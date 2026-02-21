# API Reference

Complete API documentation for luars async features.

---

## Table of Contents

- [LuaVM Methods](#luavm-methods)
  - [`register_async()`](#register_async)
  - [`execute_async()`](#execute_async)
  - [`create_async_thread()`](#create_async_thread)
  - [`register_enum()`](#register_enum)
- [Types](#types)
  - [`AsyncReturnValue`](#asyncreturnvalue)
  - [`AsyncThread`](#asyncthread)
  - [`AsyncFuture`](#asyncfuture)
  - [`LuaEnum`](#luaenum)
- [Import Paths](#import-paths)

---

## LuaVM Methods

### `register_async()`

Register a Rust async function as a Lua global function.

```rust
pub fn register_async<F, Fut>(&mut self, name: &str, f: F) -> LuaResult<()>
where
    F: Fn(Vec<LuaValue>) -> Fut + 'static,
    Fut: Future<Output = LuaResult<Vec<AsyncReturnValue>>> + 'static,
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` | `&str` | Global name of the function in Lua |
| `f` | `Fn(Vec<LuaValue>) -> Fut` | Async function factory closure |

**Closure `f` behavior:**
- Receives `Vec<LuaValue>` — arguments passed from Lua
- Returns a `Future` that resolves to `LuaResult<Vec<AsyncReturnValue>>`
- The closure itself is synchronous (`Fn`, not `async Fn`), but its return value is a Future

**Examples:**

```rust
// No arguments
vm.register_async("get_time", |_args| async move {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    Ok(vec![AsyncReturnValue::float(now)])
})?;

// With arguments
vm.register_async("add", |args| async move {
    let a = args[0].as_integer().unwrap_or(0);
    let b = args[1].as_integer().unwrap_or(0);
    Ok(vec![AsyncReturnValue::integer(a + b)])
})?;

// Multiple return values
vm.register_async("divmod", |args| async move {
    let a = args[0].as_integer().unwrap_or(0);
    let b = args[1].as_integer().unwrap_or(1);
    Ok(vec![
        AsyncReturnValue::integer(a / b),  // quotient
        AsyncReturnValue::integer(a % b),  // remainder
    ])
})?;

// Returning a UserData
vm.register_async("create_point", |args| async move {
    let x = args[0].as_number().unwrap_or(0.0);
    let y = args[1].as_number().unwrap_or(0.0);
    Ok(vec![AsyncReturnValue::userdata(Point { x, y })])
})?;
```

**Notes:**
- Registered functions can only be called from an `AsyncThread` context (i.e., Lua code executed via `execute_async()` or `create_async_thread()`)
- Calling from regular `execute()` will produce a runtime error
- The closure must be `'static` (cannot capture non-`'static` references)

---

### `execute_async()`

Compile and asynchronously execute a Lua source string. This is the simplest way to run async Lua code.

```rust
pub async fn execute_async(
    &mut self,
    source: &str,
) -> LuaResult<Vec<LuaValue>>
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `source` | `&str` | Lua source code string |

**Returns:** `LuaResult<Vec<LuaValue>>` — same return type as synchronous `execute()`.

**Examples:**

```rust
// Simple call
let results = vm.execute_async("return async_add(1, 2)").await?;
assert_eq!(results[0].as_integer(), Some(3));

// Multi-line Lua code
let results = vm.execute_async(r#"
    local a = async_fetch("https://example.com")
    local b = async_fetch("https://example.org")
    return a, b
"#).await?;

// No return value
vm.execute_async("async_log('hello')").await?;
```

**Internal implementation:**
1. Calls `vm.compile(source)` to compile to bytecode
2. Calls `vm.create_async_thread(chunk, vec![])` to create a coroutine
3. `.await` drives the coroutine to completion

---

### `create_async_thread()`

Create an `AsyncThread` from a pre-compiled Chunk. Provides lower-level control than `execute_async()`.

```rust
pub fn create_async_thread(
    &mut self,
    chunk: Chunk,
    args: Vec<LuaValue>,
) -> LuaResult<AsyncThread>
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `chunk` | `Chunk` | Pre-compiled Lua bytecode (from `vm.compile()`) |
| `args` | `Vec<LuaValue>` | Initial arguments passed to the coroutine |

**Returns:** `LuaResult<AsyncThread>` — a value implementing `Future` that can be `.await`ed.

**Use cases:**
- Reusing a compiled Chunk (avoids repeated compilation)
- Passing initial arguments to Lua code
- More control over execution

```rust
// Compile once, execute many times
let chunk = vm.compile("return async_process(...)")?;

for i in 0..10 {
    let arg = LuaValue::integer(i);
    let thread = vm.create_async_thread(chunk.clone(), vec![arg])?;
    let results = thread.await?;
    println!("Result {}: {:?}", i, results);
}
```

---

### `register_enum()`

Register a Rust enum as a Lua global table of integer constants.

```rust
pub fn register_enum<T: LuaEnum>(&mut self, name: &str) -> LuaResult<()>
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` | `&str` | Global name for the enum table in Lua |

The enum type `T` must implement `LuaEnum`, which is auto-derived by `#[derive(LuaUserData)]` on C-like enums.

**Example:**

```rust
#[derive(LuaUserData)]
enum Color { Red, Green, Blue }

#[derive(LuaUserData)]
enum HttpStatus { Ok = 200, NotFound = 404, ServerError = 500 }

vm.register_enum::<Color>("Color")?;
vm.register_enum::<HttpStatus>("Status")?;
```

```lua
-- In Lua:
print(Color.Red)      -- 0
print(Color.Green)    -- 1
print(Color.Blue)     -- 2
print(Status.Ok)      -- 200
print(Status.NotFound) -- 404

-- Use in comparisons:
local code = get_status_code()
if code == Status.NotFound then
    print("Page not found")
end
```

---

## Types

### `AsyncReturnValue`

Return value type for async functions. Because Lua strings and userdata are GC-managed, async Futures cannot directly create `LuaValue` for these types — this intermediate type solves the problem.

```rust
pub enum AsyncReturnValue {
    Value(LuaValue),        // Non-GC types: integer, float, bool, nil, lightuserdata
    String(String),          // String, converted via VM after Future completes
    UserData(LuaUserdata),   // UserData, GC-allocated via VM after Future completes
}
```

**Why is `AsyncReturnValue` needed?**

Strings and userdata in `LuaValue` are GC-allocated objects created through the VM. During async Future execution, we don't have a mutable reference to the VM (`&mut LuaVM`), so we can't directly create these types of `LuaValue`.

`AsyncReturnValue` solves this:
- Non-GC types (integer, float, bool, nil) are wrapped directly as `LuaValue` → `AsyncReturnValue::Value`
- Strings are stored as Rust `String` → `AsyncReturnValue::String`
- Userdata is stored as `LuaUserdata` → `AsyncReturnValue::UserData`
- When the Future completes, `AsyncThread` calls `vm.create_string()` or `vm.create_userdata()` to convert them to proper Lua values

**Constructors:**

| Method | Input | Description |
|--------|-------|-------------|
| `AsyncReturnValue::nil()` | — | Lua nil value |
| `AsyncReturnValue::integer(n)` | `i64` | Lua integer |
| `AsyncReturnValue::float(n)` | `f64` | Lua float |
| `AsyncReturnValue::boolean(b)` | `bool` | Lua boolean |
| `AsyncReturnValue::string(s)` | `impl Into<String>` | Lua string |
| `AsyncReturnValue::userdata(d)` | `impl UserDataTrait` | Lua userdata |

**From implementations:**

```rust
// All of these implicit conversions are implemented:
let v: AsyncReturnValue = 42i64.into();              // integer
let v: AsyncReturnValue = 3.14f64.into();            // float
let v: AsyncReturnValue = true.into();               // boolean
let v: AsyncReturnValue = "hello".into();            // string (&str)
let v: AsyncReturnValue = String::from("hi").into(); // string (String)
let v: AsyncReturnValue = LuaValue::nil().into();    // any LuaValue
let v: AsyncReturnValue = LuaUserdata::new(pt).into(); // userdata
```

---

### `AsyncThread`

Wraps a Lua coroutine as a Rust `Future`.

```rust
pub struct AsyncThread { /* ... */ }

impl Future for AsyncThread {
    type Output = LuaResult<Vec<LuaValue>>;
}
```

**Lifetime management:**
- On creation, the coroutine is automatically registered in the VM's registry to prevent GC collection
- On drop, the registry reference is automatically released
- `!Send` and `!Sync` — must be polled on the thread that created it

**Usage:**

```rust
let chunk = vm.compile("return async_fn(42)")?;
let thread = vm.create_async_thread(chunk, vec![])?;
let results = thread.await?;  // Drive coroutine to completion
```

Typically you don't need to work with `AsyncThread` directly — use `execute_async()` instead.

---

### `AsyncFuture`

Type alias representing the pending Future stored in `LuaState`.

```rust
pub type AsyncFuture = Pin<Box<dyn Future<Output = LuaResult<Vec<AsyncReturnValue>>>>>;
```

This is an internal type — regular users don't need to use it directly. Key properties:
- **`!Send`** — cannot cross threads, must run on a single-threaded runtime or `LocalSet`
- **Type-erased** — uses `dyn Future` to support arbitrary async functions

---

### `LuaEnum`

Trait for Rust enums that can be exported to Lua as a table of integer constants.

```rust
pub trait LuaEnum {
    fn variants() -> &'static [(&'static str, i64)];
    fn enum_name() -> &'static str;
}
```

Auto-derived by `#[derive(LuaUserData)]` on C-like enums (enums with no data fields). Each variant becomes a key-value pair where the value is the variant's discriminant.

---

## Import Paths

```rust
// Recommended: import from crate root
use luars::{AsyncReturnValue, AsyncThread, AsyncFuture, LuaEnum};
use luars::{LuaVM, LuaResult, LuaValue, LuaUserdata, Stdlib};
use luars::lua_vm::SafeOption;

// Or import from module paths
use luars::lua_vm::async_thread::{AsyncReturnValue, AsyncThread, AsyncFuture};
use luars::lua_value::userdata_trait::LuaEnum;
```

---

## Related Documentation

- [Getting Started](./01-getting-started.md) — Introductory tutorial
- [Examples](./03-examples.md) — More code examples
- [Internal Architecture](./04-architecture.md) — Implementation details
