# Getting Started

This guide covers the current embedding model in luars.

## The Three Runtime Types

luars now separates responsibilities across three layers:

| Type | Role | Typical user |
|------|------|--------------|
| `Lua` | High-level embedding API | Application / library host code |
| `LuaState` | Per-thread execution context | Rust callbacks, coroutines, advanced host code |
| `GlobalState` | Low-level owner for GC, registry, globals, and thread allocation | Advanced internal or escape-hatch usage |

For normal host-side code, start with `Lua`.

## Creating a Runtime

```rust
use luars::{Lua, SafeOption};

let mut lua = Lua::new(SafeOption::default());
```

`SafeOption` controls runtime limits such as call depth, stack size, GC memory, and optional instruction budgeting.

```rust
use luars::{Lua, SafeOption};

let options = SafeOption {
    max_call_depth: 100,
    max_stack_size: 10_000,
    max_gc_memory: 16 * 1024 * 1024,
    max_instruction_count: 1_000_000,
};

let mut lua = Lua::new(options);
```

## Loading Standard Libraries

```rust
use luars::{Lua, LuaApi, SafeOption, Stdlib};

let mut lua = Lua::new(SafeOption::default());
lua.open_stdlib(Stdlib::All)?;
```

You can load libraries selectively as well:

```rust
lua.open_stdlib(Stdlib::Basic)?;
lua.open_stdlib(Stdlib::String)?;
lua.open_stdlib(Stdlib::Math)?;
```

## Running Code

The high-level API is chunk-based. Load source, then choose whether to execute it or evaluate typed results.

```rust
use luars::{Lua, LuaApi, SafeOption, Stdlib};

let mut lua = Lua::new(SafeOption::default());
lua.open_stdlib(Stdlib::All)?;

lua.load("answer = 40 + 2").exec()?;
let answer: i64 = lua.load("return answer").eval()?;

assert_eq!(answer, 42);
# Ok::<(), luars::LuaError>(())
```

## Accessing the Main State

`LuaState` is the real execution context. You usually interact with it in two places:

- inside registered Rust callbacks
- in advanced code that works directly with `GlobalState`

```rust
use luars::{GlobalState, SafeOption};

let mut global = GlobalState::new(SafeOption::default());
let state = global.main_state();
let _ = state;
```

`Lua` intentionally keeps that lower-level owner behind its high-level API. Prefer `Lua` for top-level embedding code, and reach for `LuaState` only when you need direct `load`, `call`, `pcall`, coroutine, or registry-adjacent behavior.

## When to Use GlobalState

`GlobalState` is no longer the main host-facing API. It remains the low-level owner for:

- GC control
- registry access
- global environment access
- low-level value and ref creation
- thread allocation and runtime internals

If you are writing ordinary embedding code, treat `GlobalState` as infrastructure, not your primary surface.

## Minimal Example

```rust
use luars::{Lua, LuaApi, SafeOption, Stdlib};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut lua = Lua::new(SafeOption::default());
    lua.open_stdlib(Stdlib::All)?;

    let value: i64 = lua
        .load(
            r#"
            local function factorial(n)
                if n <= 1 then return 1 end
                return n * factorial(n - 1)
            end
            return factorial(10)
            "#,
        )
        .eval()?;

    println!("10! = {}", value);
    Ok(())
}
```

## Next

- [Executing Code](02-ExecutingCode.md) covers `load`, typed evaluation, low-level `LuaState` calls, and compilation.
- [Working with Values](03-WorkingWithValues.md) covers globals, tables, strings, and userdata.
