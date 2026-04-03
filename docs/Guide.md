# luars High-Level Guide

This guide documents the high-level embedding API exposed through `luars::Lua`.

If you are embedding Lua into an application, start here. The examples below intentionally avoid low-level `LuaVM`, raw stack manipulation, and direct `LuaValue` plumbing.

## Installation

```toml
[dependencies]
luars = "0.18"
```

The `#[derive(LuaUserData)]` and `#[lua_methods]` macros are re-exported by `luars`, so you usually do not need to depend on `luars-derive` directly.

## 1. Create a runtime

```rust
use luars::{Lua, SafeOption, Stdlib};

let mut lua = Lua::new(SafeOption::default());
lua.load_stdlibs(Stdlib::All)?;
```

## 2. Execute chunks

```rust
lua.load(
    r#"
    local total = 0
    for i = 1, 100 do
        total = total + i
    end
    result = total
    "#,
)
.exec()?;

let result: i64 = lua.load("return result").eval()?;
assert_eq!(result, 5050);
```

Use these entry points in normal host code:

- `exec()` for chunks with side effects
- `eval()` for one typed return value
- `eval_multi()` for tuple-style results

If the chunk may call async Rust functions, the same builder exposes:

- `exec_async()`
- `eval_async()`
- `eval_multi_async()`

## 3. Register Rust functions

```rust
lua.register_function("discount", |price_cents: i64, percent: i64| -> i64 {
    price_cents - (price_cents * percent / 100)
})?;

let value: i64 = lua.load("return discount(2000, 15)").eval()?;
assert_eq!(value, 1700);
```

Async callbacks use the same typed style:

```rust
lua.register_async_function("double_async", |value: i64| async move {
    Ok(value * 2)
})?;

let result: i64 = lua.load("return double_async(21)").eval_async().await?;
assert_eq!(result, 42);
```

## 4. Call Lua from Rust

```rust
lua.load(
    r#"
    function classify(score)
        if score >= 90 then
            return "excellent"
        elseif score >= 60 then
            return "pass"
        else
            return "retry"
        end
    end
    "#,
)
.exec()?;

let label: String = lua.call_global1("classify", 92_i64)?;
assert_eq!(label, "excellent");
```

Async host code can use `call_async()`, `call_async1()`, `call_async_global()`, and `call_async_global1()`.

## 5. Exchange tables and globals

```rust
let request = lua.create_table()?;
request.set("path", "/orders")?;
request.set("method", "GET")?;

let headers = lua.create_table_from([("x-request-id", "req-42"), ("x-env", "dev")])?;
request.set("headers", headers)?;
lua.globals().set("request", request)?;

let method: String = lua.load("return request.method").eval()?;
assert_eq!(method, "GET");
```

The high-level table API is the default way to exchange structured data with Lua.

## 6. Expose Rust types

```rust
use luars::{LuaUserData, lua_methods};

#[derive(LuaUserData)]
struct Counter {
    pub count: i64,
}

#[lua_methods]
impl Counter {
    pub fn new(count: i64) -> Self {
        Self { count }
    }

    pub fn inc(&mut self, delta: i64) {
        self.count += delta;
    }

    pub fn get(&self) -> i64 {
        self.count
    }
}

lua.register_type::<Counter>("Counter")?;
```

More detail is available in [UserGuide.md](UserGuide.md).

## 7. Use scope for borrowed values

`scope(...)` lets you temporarily expose borrowed Rust data to Lua without forcing it into a `'static` lifetime.

```rust
let formatted: String = lua.scope(|scope| {
    let prefix = String::from("order:");
    let render = scope.create_function_with(&prefix, |prefix: &String, id: i64| {
        format!("{prefix}{id}")
    })?;

    scope.globals().set("render", &render)?;
    scope.load("return render(42)").eval()
})?;

assert_eq!(formatted, "order:42");
```

## 8. Use sandboxed chunks

When the `sandbox` feature is enabled, the high-level API can execute chunks in an isolated `_ENV`:

```rust
use luars::SandboxConfig;

let mut sandbox = SandboxConfig::default();
lua.sandbox_insert_global(&mut sandbox, "answer", 42_i64)?;

let answer: i64 = lua.eval_sandboxed("return answer", &sandbox)?;
assert_eq!(answer, 42);
assert!(lua.get_global::<i64>("answer")?.is_none());
```

`load_sandboxed()` returns a chunk builder, which is useful when you want a sandboxed script to return a function or table that you keep on the Rust side.

## Examples

- [../examples/luars-example/src/main.rs](../examples/luars-example/src/main.rs) for globals, userdata, and scope
- [../examples/rules-engine-demo/src/main.rs](../examples/rules-engine-demo/src/main.rs) for host functions and table exchange
- [../examples/http-server/src/main.rs](../examples/http-server/src/main.rs) for async request handling with sandboxed Lua handlers
- [../examples/rust-bind-bench/src/main.rs](../examples/rust-bind-bench/src/main.rs) for repeated userdata calls
