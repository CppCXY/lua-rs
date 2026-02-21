# Getting Started

Get up and running with luars async in 5 minutes.

## Dependencies

Add the following to your `Cargo.toml`:

```toml
[dependencies]
luars = { version = "0.6" }
tokio = { version = "1", features = ["rt", "macros", "time"] }
```

> **Note**: luars async features don't require an extra feature flag — they are part of the core API.

## Your First Async Function

```rust
use luars::{LuaVM, LuaResult, Stdlib, AsyncReturnValue};
use luars::lua_vm::SafeOption;
use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() -> LuaResult<()> {
    // 1. Create VM and load standard library
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    // 2. Register an async function
    vm.register_async("sleep", |args| async move {
        let secs = args[0].as_number().unwrap_or(1.0);
        tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        Ok(vec![AsyncReturnValue::boolean(true)])
    })?;

    // 3. Call it from Lua
    let results = vm.execute_async(r#"
        print("sleeping...")
        sleep(0.5)
        print("done!")
        return "finished"
    "#).await?;

    println!("Lua returned: {:?}", results[0].as_str());
    Ok(())
}
```

## Three-Step Flow

```text
  ┌─────────────────────────────┐
  │ 1. register_async(name, f)  │  Register async function
  └──────────────┬──────────────┘
                 ▼
  ┌─────────────────────────────┐
  │ 2. execute_async()   │  Execute Lua code asynchronously
  │    or create_async_thread() │
  └──────────────┬──────────────┘
                 ▼
  ┌─────────────────────────────┐
  │ 3. .await to get results    │  Future drives the coroutine
  └─────────────────────────────┘
```

### Step 1: Register Async Functions

Use `vm.register_async(name, closure)`. The closure receives `Vec<LuaValue>` arguments and returns a `Future<Output = LuaResult<Vec<AsyncReturnValue>>>`.

```rust
vm.register_async("read_file", |args| async move {
    let path = args[0].as_str().unwrap_or("").to_string();
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => Ok(vec![
            AsyncReturnValue::string(content),
            AsyncReturnValue::nil(),
        ]),
        Err(e) => Ok(vec![
            AsyncReturnValue::nil(),
            AsyncReturnValue::string(e.to_string()),
        ]),
    }
})?;
```

### Step 2: Execute Lua Code Asynchronously

Two equivalent approaches:

```rust
// Approach A: Concise — compile + execute in one step
let results = vm.execute_async("return read_file('config.txt')").await?;

// Approach B: Manual — compile first, then create AsyncThread
let chunk = vm.compile("return read_file('config.txt')")?;
let thread = vm.create_async_thread(chunk, vec![])?;
let results = thread.await?;
```

### Step 3: Handle Results

`.await` returns `LuaResult<Vec<LuaValue>>` — the exact same type as synchronous `execute_string()`.

```rust
let results = vm.execute_async("return read_file('config.txt')").await?;
let content = results[0].as_str().unwrap_or("(nil)");
println!("File content: {}", content);
```

## Important Constraints

1. **Must use async execution**: Registered async functions can only be called via `execute_async()` or `create_async_thread()`. Calling async functions from regular `execute_string()` will cause an error.

2. **`LuaVM` is `!Send`**: Cannot be moved across threads. For multi-threading, use the thread-per-VM pattern (see [Multi-VM Patterns](./05-multi-vm.md)).

3. **String and UserData return values**: Lua strings and userdata are GC-managed, so async Futures cannot directly create `LuaValue` for them. Use `AsyncReturnValue::string()` or `AsyncReturnValue::userdata()` instead — the framework converts them automatically after the Future completes.

4. **tokio runtime**: Use `current_thread` flavor, since `LuaVM` is `!Send`.

## Next Steps

- [API Reference](./02-api-reference.md) — Complete documentation of all types and methods
- [Examples](./03-examples.md) — More code examples
