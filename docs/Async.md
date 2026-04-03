# luars High-Level Async And Sandbox Notes

This document describes the async and sandbox surface exposed through the high-level `Lua` API.

## Async support

The high-level API supports typed async registration and async execution.

```rust
lua.register_async_function("fetch_len", |url: String| async move {
    let body = reqwest::get(&url).await?.text().await?;
    Ok(body.len() as i64)
})?;

let len: i64 = lua.load("return fetch_len('https://example.com')").eval_async().await?;
```

Available async entry points:

- `lua.exec_async(source)`
- `lua.eval_async(source)`
- `lua.eval_multi_async(source)`
- `lua.call_async(function, args)`
- `lua.call_async1(function, args)`
- `lua.call_async_global(name, args)`
- `lua.call_async_global1(name, args)`
- `lua.load(...).exec_async()` and related chunk-builder methods

These wrappers stay typed: arguments still use `IntoLua`, and results still use `FromLua` / `FromLuaMulti`.

## Sandbox support

When the `sandbox` feature is enabled, the high-level API exposes the existing isolated `_ENV` support:

```rust
use luars::SandboxConfig;

let mut sandbox = SandboxConfig::default();
lua.sandbox_insert_global(&mut sandbox, "answer", 42_i64)?;

let answer: i64 = lua.eval_sandboxed("return answer", &sandbox)?;
assert_eq!(answer, 42);
```

Available sandbox entry points:

- `lua.execute_sandboxed(source, config)`
- `lua.eval_sandboxed(source, config)`
- `lua.eval_multi_sandboxed(source, config)`
- `lua.load_sandboxed(source, config)`
- `lua.sandbox_insert_global(config, name, value)`
- `lua.sandbox_capture_global(config, name)`

`load_sandboxed()` is the high-level API you want when a script should return a function or table that Rust keeps for later use.

## Important boundary

Runtime limits in `SandboxConfig` apply to `execute_sandboxed()` calls directly. When you use `load_sandboxed()` and keep the returned function for later calls, environment isolation remains, but runtime limits are not automatically re-applied on every later invocation.

That mirrors the current lower-level runtime semantics.
