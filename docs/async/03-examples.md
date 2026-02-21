# Examples

Async usage examples from simple to complex.

---

## Table of Contents

- [Basics: Returning Different Value Types](#basics-returning-different-value-types)
- [Returning UserData from Async Functions](#returning-userdata-from-async-functions)
- [Error Handling](#error-handling)
- [Async File I/O](#async-file-io)
- [Async Sleep / Timers](#async-sleep--timers)
- [Multiple Async Calls in Sequence](#multiple-async-calls-in-sequence)
- [Mixing with Synchronous Lua Code](#mixing-with-synchronous-lua-code)
- [Calling Async Functions in Loops](#calling-async-functions-in-loops)
- [Lua-side pcall Error Protection](#lua-side-pcall-error-protection)
- [Pre-compiled Chunk Reuse](#pre-compiled-chunk-reuse)
- [Exporting Enums to Lua](#exporting-enums-to-lua)

---

## Basics: Returning Different Value Types

### Returning integers

```rust
vm.register_async("async_add", |args| async move {
    let a = args[0].as_integer().unwrap_or(0);
    let b = args[1].as_integer().unwrap_or(0);
    Ok(vec![AsyncReturnValue::integer(a + b)])
})?;

let results = vm.execute_async("return async_add(10, 20)").await?;
assert_eq!(results[0].as_integer(), Some(30));
```

### Returning strings

```rust
vm.register_async("greet", |args| async move {
    let name = args[0].as_str().unwrap_or("world").to_string();
    Ok(vec![AsyncReturnValue::string(format!("Hello, {}!", name))])
})?;

let results = vm.execute_async(r#"return greet("Lua")"#).await?;
assert_eq!(results[0].as_str(), Some("Hello, Lua!"));
```

> **Note**: When capturing `as_str()` references in closures, you must first call `.to_string()` to create an owned `String`, because `LuaValue` references cannot cross `.await` points.

### Returning multiple values

Lua natively supports multiple return values, and async functions work the same way:

```rust
vm.register_async("divmod", |args| async move {
    let a = args[0].as_integer().unwrap_or(0);
    let b = args[1].as_integer().unwrap_or(1);
    Ok(vec![
        AsyncReturnValue::integer(a / b),
        AsyncReturnValue::integer(a % b),
    ])
})?;

// Lua side
let results = vm.execute_async(r#"
    local q, r = divmod(17, 5)
    return q, r
"#).await?;
assert_eq!(results[0].as_integer(), Some(3));
assert_eq!(results[1].as_integer(), Some(2));
```

### Returning nil

```rust
vm.register_async("maybe_nil", |_args| async move {
    Ok(vec![AsyncReturnValue::nil()])
})?;
```

> **Important**: Returning an empty vec `Ok(vec![])` means "no return values" (not nil). To explicitly return nil, use `vec![AsyncReturnValue::nil()]`.

---

## Returning UserData from Async Functions

Async functions can return UserData objects. The data is stored during the Future's execution and GC-allocated when the Future completes.

```rust
use luars::{LuaUserData, lua_methods};

#[derive(LuaUserData)]
#[lua_impl(Display)]
struct FetchResult {
    pub status: i64,
    pub body: String,
}

#[lua_methods]
impl FetchResult {
    pub fn is_ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

// Register an async function that returns UserData
vm.register_async("fetch", |args| async move {
    let url = args[0].as_str().unwrap_or("").to_string();
    // ... perform async HTTP request ...
    let result = FetchResult { status: 200, body: "Hello".to_string() };
    Ok(vec![AsyncReturnValue::userdata(result)])
})?;
```

```lua
-- In Lua: access fields and call methods on the returned userdata
local resp = fetch("https://example.com")
print(resp.status)    -- 200
print(resp.body)      -- "Hello"
print(resp:is_ok())   -- true
```

You can also mix userdata with other return types:

```rust
vm.register_async("fetch_with_timing", |args| async move {
    let start = std::time::Instant::now();
    let result = FetchResult { status: 200, body: "...".to_string() };
    let elapsed = start.elapsed().as_secs_f64();
    Ok(vec![
        AsyncReturnValue::userdata(result),
        AsyncReturnValue::float(elapsed),
    ])
})?;
```

---

## Error Handling

### Returning errors from Rust

```rust
use luars::lua_vm::LuaError;

vm.register_async("safe_divide", |args| async move {
    let a = args[0].as_number().unwrap_or(0.0);
    let b = args[1].as_number().unwrap_or(0.0);
    if b == 0.0 {
        return Err(LuaError::RuntimeError);
    }
    Ok(vec![AsyncReturnValue::float(a / b)])
})?;

// Catch error on the Rust side
let result = vm.execute_async("return safe_divide(1, 0)").await;
assert!(result.is_err());
```

### Go-style error convention

A more recommended approach is to return `(value, error)` tuples following the Lua/Go convention:

```rust
vm.register_async("read_file", |args| async move {
    let path = args[0].as_str().unwrap_or("").to_string();
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => Ok(vec![
            AsyncReturnValue::string(content),
            AsyncReturnValue::nil(),       // no error
        ]),
        Err(e) => Ok(vec![
            AsyncReturnValue::nil(),       // no result
            AsyncReturnValue::string(e.to_string()),  // error message
        ]),
    }
})?;
```

```lua
-- Lua side
local content, err = read_file("config.txt")
if err then
    print("Error: " .. err)
else
    print("Content: " .. content)
end
```

---

## Async File I/O

```rust
// Async file read
vm.register_async("read_file", |args| async move {
    let path = args[0].as_str().unwrap_or("").to_string();
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => Ok(vec![AsyncReturnValue::string(content), AsyncReturnValue::nil()]),
        Err(e) => Ok(vec![AsyncReturnValue::nil(), AsyncReturnValue::string(e.to_string())]),
    }
})?;

// Async file write
vm.register_async("write_file", |args| async move {
    let path = args[0].as_str().unwrap_or("").to_string();
    let content = args[1].as_str().unwrap_or("").to_string();
    match tokio::fs::write(&path, content.as_bytes()).await {
        Ok(()) => Ok(vec![AsyncReturnValue::boolean(true), AsyncReturnValue::nil()]),
        Err(e) => Ok(vec![AsyncReturnValue::nil(), AsyncReturnValue::string(e.to_string())]),
    }
})?;
```

```lua
-- Lua side
local content, err = read_file("input.txt")
if not err then
    local ok, err2 = write_file("output.txt", string.upper(content))
    if ok then print("Written!") end
end
```

---

## Async Sleep / Timers

```rust
vm.register_async("sleep", |args| async move {
    let secs = args[0].as_number().unwrap_or(1.0);
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
    Ok(vec![AsyncReturnValue::boolean(true)])
})?;
```

```lua
-- Lua side: sleep doesn't block other coroutines/workers
print("start")
sleep(1.5)
print("1.5 seconds later")
```

---

## Multiple Async Calls in Sequence

Within the same Lua code block, multiple async function calls execute sequentially:

```rust
vm.register_async("async_double", |args| async move {
    let n = args[0].as_integer().unwrap_or(0);
    // Simulate some async work
    tokio::time::sleep(Duration::from_millis(10)).await;
    Ok(vec![AsyncReturnValue::integer(n * 2)])
})?;

let results = vm.execute_async(r#"
    local a = async_double(5)    -- 10
    local b = async_double(a)    -- 20
    local c = async_double(b)    -- 40
    return c
"#).await?;
assert_eq!(results[0].as_integer(), Some(40));
```

Each `async_double` call causes the Lua coroutine to yield, waits for the Future to complete, then resumes. From Lua's perspective, this is indistinguishable from calling a normal function.

---

## Mixing with Synchronous Lua Code

Async functions seamlessly integrate with all Lua features:

```rust
vm.register_async("async_fetch", |args| async move {
    let key = args[0].as_str().unwrap_or("?").to_string();
    Ok(vec![AsyncReturnValue::string(format!("data_{}", key))])
})?;

let results = vm.execute_async(r#"
    -- Regular Lua function
    local function transform(s)
        return string.upper(s) .. "!"
    end

    -- Call async function, then process with regular function
    local raw = async_fetch("test")
    local result = transform(raw)

    -- Use in table constructors
    local t = { value = async_fetch("key"), count = 42 }

    return result, t.value
"#).await?;

assert_eq!(results[0].as_str(), Some("DATA_TEST!"));
assert_eq!(results[1].as_str(), Some("data_key"));
```

---

## Calling Async Functions in Loops

```rust
vm.register_async("async_inc", |args| async move {
    let n = args[0].as_integer().unwrap_or(0);
    Ok(vec![AsyncReturnValue::integer(n + 1)])
})?;

let results = vm.execute_async(r#"
    local sum = 0
    for i = 1, 5 do
        sum = sum + async_inc(i)
    end
    return sum   -- (1+1) + (2+1) + (3+1) + (4+1) + (5+1) = 20
"#).await?;
assert_eq!(results[0].as_integer(), Some(20));
```

Each loop iteration triggers yield → poll → resume, but this is completely transparent to the Lua code.

---

## Lua-side pcall Error Protection

Errors from async functions can be caught by Lua's `pcall` / `xpcall`:

```rust
vm.register_async("risky_op", |_args| async move {
    Err(luars::lua_vm::LuaError::RuntimeError)
})?;

let results = vm.execute_async(r#"
    local ok, err = pcall(risky_op)
    if not ok then
        return "caught: " .. tostring(err)
    end
    return "success"
"#).await?;
// results[0] will contain the error message
```

---

## Pre-compiled Chunk Reuse

If you need to execute the same Lua code repeatedly (e.g., handling multiple requests), you can pre-compile once and reuse:

```rust
vm.register_async("process", |args| async move {
    let n = args[0].as_integer().unwrap_or(0);
    tokio::time::sleep(Duration::from_millis(1)).await;
    Ok(vec![AsyncReturnValue::integer(n * n)])
})?;

// Compile once
let chunk = vm.compile("return process(...)")?;

// Execute many times
for i in 0..5 {
    let arg = LuaValue::integer(i);
    let thread = vm.create_async_thread(chunk.clone(), vec![arg])?;
    let results = thread.await?;
    println!("process({}) = {}", i, results[0].as_integer().unwrap());
}
// Output: process(0) = 0, process(1) = 1, ..., process(4) = 16
```

---

## Exporting Enums to Lua

C-like Rust enums can be exported to Lua as tables of integer constants:

```rust
#[derive(LuaUserData)]
enum Direction {
    North,    // 0
    South,    // 1
    East,     // 2
    West,     // 3
}

#[derive(LuaUserData)]
enum Priority {
    Low = 1,
    Medium = 5,
    High = 10,
    Critical = 100,
}

vm.register_enum::<Direction>("Direction")?;
vm.register_enum::<Priority>("Priority")?;
```

```lua
-- In Lua: enums are just tables of integers
print(Direction.North)   -- 0
print(Direction.West)    -- 3
print(Priority.High)     -- 10

-- Use in logic
local function process(dir)
    if dir == Direction.North then
        return "Going north!"
    end
    return "Going somewhere else"
end

print(process(Direction.North))  -- "Going north!"

-- Iterate over all variants
for name, value in pairs(Priority) do
    print(name, value)
end
```

---

## Next Steps

- [Internal Architecture](./04-architecture.md) — Understand the implementation details behind async
- [Multi-VM Patterns](./05-multi-vm.md) — Using async in multi-threaded servers
- [HTTP Server Example](./06-http-server.md) — Complete real-world project walkthrough
