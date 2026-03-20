# luars Async Documentation

This directory contains the complete documentation for luars async features.

## Documentation Index

| Document | Contents |
|----------|----------|
| [Getting Started](async/01-getting-started.md) | 5-minute async quickstart, your first async function |
| [API Reference](async/02-api-reference.md) | Detailed description of all async types and methods |
| [Examples](async/03-examples.md) | Code examples from simple to complex |
| [Internal Architecture](async/04-architecture.md) | Coroutine↔Future bridging implementation details |
| [Multi-VM Patterns](async/05-multi-vm.md) | Design patterns for multi-LuaVM concurrent processing |
| [HTTP Server Example](async/06-http-server.md) | Complete async HTTP server walkthrough |

## Core Concept Overview

## Typed-First API

For most embedding code, prefer `register_async_typed` over the raw `register_async` API.

```rust
use luars::{LuaVM, SafeOption, Stdlib};

let mut vm = LuaVM::new(SafeOption::default());
vm.open_stdlib(Stdlib::All)?;

vm.register_async_typed("fetch_len", |url: String| async move {
      let body = reqwest::get(&url).await?.text().await?;
      Ok(body.len() as i64)
})?;

let results = vm.execute_async("return fetch_len('https://example.com')").await?;
assert!(results[0].as_integer().unwrap_or(0) > 0);
```

Typed async callbacks use:

- `FromLua` to decode arguments
- `IntoAsyncLua` to encode awaited return values
- tuple returns for Lua multi-return values
- `UserDataRef<T>` for typed userdata parameters

Example with multiple return values:

```rust
vm.register_async_typed("split_stats", |s: String| async move {
      Ok((s.len() as i64, s.to_uppercase()))
})?;
```

Keep `register_async` for cases where you already want to manually work with `Vec<LuaValue>` and `Vec<AsyncReturnValue>`.

```text
Rust async runtime (tokio)
  └── AsyncThread::poll()          ← Driver: implements Future trait
        ├── has pending future? → poll it
        │     ├── Pending → return Poll::Pending
        │     └── Ready(result) → resume(result) → continue checking
        └── no pending future → resume(args)
              ├── coroutine finished → return Poll::Ready
              ├── async yield (sentinel) → take future, poll it
              └── normal yield → wake & return Pending
```

**Key point**: From Lua's perspective, async functions behave exactly like normal synchronous functions. The async yield/resume is completely transparent to Lua code.
