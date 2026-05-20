# Executing Code

This guide describes the current execution model.

## High-Level Host API: `Lua`

For embedding code, prefer `Lua` plus the `LuaApi` trait.

### Load Then Execute

```rust
use luars::{Lua, LuaApi, SafeOption};

let mut lua = Lua::new(SafeOption::default());
lua.load("answer = 40 + 2").exec()?;
# Ok::<(), luars::LuaError>(())
```

### Typed Evaluation

```rust
use luars::{Lua, LuaApi, SafeOption};

let mut lua = Lua::new(SafeOption::default());
let answer: i64 = lua.load("return 21 * 2").eval()?;
assert_eq!(answer, 42);
# Ok::<(), luars::LuaError>(())
```

### Multiple Return Values

```rust
use luars::{Lua, LuaApi, SafeOption};

let mut lua = Lua::new(SafeOption::default());
let values: (i64, String, bool) = lua.load("return 1, 'two', true").eval_multi()?;
assert_eq!(values, (1, "two".to_string(), true));
# Ok::<(), luars::LuaError>(())
```

### Naming a Chunk

```rust
let value: i64 = lua
    .load("return 42")
    .set_name("init.lua")
    .eval()?;
# Ok::<(), luars::LuaError>(())
```

Use chunk names when you want better error messages and tracebacks.

## Low-Level Execution API: `LuaState`

`LuaState` is the lower-level execution context. Use it when you need direct `LuaValue` control, low-level calling, `pcall`, coroutine operations, or file-based loading.

```rust
use luars::{GlobalState, LuaValue, SafeOption, Stdlib};

let mut global = GlobalState::new(SafeOption::default());
global.open_stdlib(Stdlib::All)?;

let state = global.main_state();
state.execute("function add(a, b) return a + b end")?;

let func = state.get_global("add")?.unwrap();
let results = state.call(func, vec![LuaValue::integer(3), LuaValue::integer(4)])?;
assert_eq!(results[0].as_integer(), Some(7));
# Ok::<(), luars::LuaError>(())
```

### Available Low-Level Entry Points

- `state.load(source)` compiles to a callable `LuaValue`
- `state.load_with_name(source, chunk_name)` does the same with a custom chunk name
- `state.execute(source)` is a convenience wrapper for `load + call`
- `state.dofile(path)` reads, compiles, and executes a file
- `state.call(func, args)` calls an arbitrary Lua function value
- `state.call_global(name, args)` looks up a global and calls it
- `state.pcall(func, args)` performs a protected call

## Compiling Without Immediate Execution

If you need a compiled chunk for repeated use, call `load` on `LuaState` or use `Lua::load` and keep the returned `Chunk`.

```rust
let func = global.main_state().load("return 40 + 2")?;
let values = global.main_state().call(func, vec![])?;
assert_eq!(values[0].as_integer(), Some(42));
# Ok::<(), luars::LuaError>(())
```

For advanced tooling or caching, `GlobalState` still exposes raw compilation helpers such as `compile`, `compile_with_name`, and `load_proto_from_file`.

## Return Values

At the low level, Lua execution returns `Vec<LuaValue>`.

```rust
let results = global.main_state().execute("return 1, 'two', true, nil")?;

assert_eq!(results[0].as_integer(), Some(1));
assert_eq!(results[1].as_str(), Some("two"));
assert_eq!(results[2].as_boolean(), Some(true));
assert!(results[3].is_nil());
# Ok::<(), luars::LuaError>(())
```

At the high level, prefer `eval`, `eval_multi`, `call_global`, and typed wrappers so conversion happens at the API boundary.

## Error Handling

High-level API:

```rust
match lua.load("error('boom')").exec() {
    Ok(()) => {}
    Err(err) => {
        let full = lua.get_error_message(err);
        eprintln!("{}", full);
    }
}
```

Low-level API:

```rust
match global.main_state().execute("error('boom')") {
    Ok(_) => {}
    Err(err) => {
        let full = global.get_full_error(err);
        eprintln!("{}", full);
    }
}
```

## Async and Sandbox Execution

- For high-level async embedding, use `LuaAsyncApi` on `Lua`.
- For low-level async execution, use `LuaState::execute_async`, `LuaState::call_async`, and `LuaState::create_async_call_handle_global`.
- For sandboxed execution, use `LuaSandboxApi` on `Lua` or `LuaState::load_sandboxed` / `LuaState::execute_sandboxed`.

## Next

- [Working with Values](03-WorkingWithValues.md) covers globals, tables, strings, and userdata.
- [Rust Functions in Lua](04-RustFunctions.md) covers function registration and callbacks.
