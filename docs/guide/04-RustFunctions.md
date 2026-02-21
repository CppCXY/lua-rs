# Rust Functions in Lua

This guide explains how to expose Rust functions to Lua — from simple function pointers to closures that capture state.

## CFunction (Function Pointers)

The simplest way to register a Rust function. A `CFunction` is a bare function pointer:

```rust
use luars::lua_vm::{LuaVM, SafeOption, LuaState};
use luars::{LuaResult, LuaValue, Stdlib};

fn my_add(state: &mut LuaState) -> LuaResult<usize> {
    // Get arguments from the Lua stack
    let a = state.get_arg(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let b = state.get_arg(2).and_then(|v| v.as_number()).unwrap_or(0.0);

    // Push the result
    state.push_value(LuaValue::float(a + b))?;

    // Return the number of return values
    Ok(1)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::Basic)?;

    // Register as a global function
    vm.set_global("my_add", LuaValue::cfunction(my_add))?;

    vm.execute(r#"
        print(my_add(3, 4))    -- 7.0
    "#)?;

    Ok(())
}
```

### CFunction Signature

```rust
pub type CFunction = fn(&mut LuaState) -> LuaResult<usize>;
```

- Receives a `&mut LuaState` (the current thread's execution context)
- Returns `LuaResult<usize>` — the number of values pushed onto the stack
- Cannot capture variables (it's a plain `fn` pointer, not a closure)

### Accessing Arguments

Inside a CFunction, use `LuaState` methods to read arguments:

```rust
fn example(state: &mut LuaState) -> LuaResult<usize> {
    // Number of arguments passed
    let n = state.arg_count();

    // Get individual arguments (1-based indexing)
    let arg1 = state.get_arg(1);  // Option<LuaValue>
    let arg2 = state.get_arg(2);

    // All arguments as a Vec
    let all_args = state.get_args();

    Ok(0)
}
```

### Returning Values

Push values onto the stack and return the count:

```rust
fn multi_return(state: &mut LuaState) -> LuaResult<usize> {
    state.push_value(LuaValue::integer(1))?;
    state.push_value(LuaValue::integer(2))?;
    state.push_value(LuaValue::integer(3))?;
    Ok(3)  // returning 3 values
}
```

```lua
local a, b, c = multi_return()  -- 1, 2, 3
```

## RClosure (Rust Closures)

`RClosure` allows registering Rust closures that **capture state** — something CFunction cannot do since it's a bare function pointer.

### create_closure

```rust
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

let counter = Arc::new(AtomicUsize::new(0));
let counter_clone = counter.clone();

// Create a closure that captures `counter_clone`
let func = vm.create_closure(move |state: &mut LuaState| {
    let n = counter_clone.fetch_add(1, Ordering::SeqCst);
    state.push_value(LuaValue::integer(n as i64))?;
    Ok(1)
})?;

vm.set_global("next_id", func)?;

vm.execute(r#"
    print(next_id())  -- 0
    print(next_id())  -- 1
    print(next_id())  -- 2
"#)?;

// Rust can also read the counter
println!("Counter = {}", counter.load(Ordering::SeqCst)); // 3
```

### create_closure (on LuaState)

The same API is available on `LuaState`, useful inside callbacks:

```rust
let state = vm.main_state();
let func = state.create_closure(|state: &mut LuaState| {
    let msg = state.get_arg(1)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    println!("Lua says: {}", msg);
    Ok(0)
})?;
state.set_global("rust_print", func)?;
```

### create_closure_with_upvalues

Create a closure with Lua upvalues (accessible within the closure):

```rust
let upvalue = vm.create_string("prefix")?;
let func = vm.create_closure_with_upvalues(
    |state: &mut LuaState| {
        // Access upvalues, arguments, etc.
        Ok(0)
    },
    vec![upvalue],
)?;
```

## Calling Lua Functions from Rust

You can call Lua functions from Rust using `call`, `pcall`, or `xpcall`:

### call

Calls a function directly. Errors propagate as `LuaError`:

```rust
vm.execute(r#"
    function greet(name)
        return "Hello, " .. name .. "!"
    end
"#)?;

let greet = vm.get_global("greet")?.unwrap();
let state = vm.main_state();
let name = state.create_string("World")?;
let results = state.call(greet, vec![name])?;
println!("{}", results[0].as_str().unwrap()); // "Hello, World!"
```

### pcall (Protected Call)

Catches errors instead of propagating them:

```rust
let func = vm.get_global("might_fail")?.unwrap();
let state = vm.main_state();
let (ok, results) = state.pcall(func, vec![])?;

if ok {
    println!("Success: {:?}", results);
} else {
    println!("Error: {:?}", results[0].as_str());
}
```

### xpcall (Protected Call with Error Handler)

Like `pcall` but with a custom error handler:

```rust
let func = vm.get_global("might_fail")?.unwrap();
let handler = vm.get_global("error_handler")?.unwrap();
let state = vm.main_state();
let (ok, results) = state.xpcall(func, vec![], handler)?;
```

## CFunction vs RClosure

| Feature | CFunction | RClosure |
|---------|-----------|----------|
| Type | `fn` pointer | `Box<dyn Fn>` |
| Capture state | No | Yes |
| Performance | Direct call | Vtable indirect call (~2-5ns overhead) |
| GC managed | No (light) | Yes |
| Use case | Stateless functions, stdlib | Stateful callbacks, WASM interop |
| Created via | `LuaValue::cfunction(f)` | `vm.create_closure(\|s\| ...)` |

Both appear as `type(f) == "function"` in Lua — they are indistinguishable from the Lua side.

## Building a Function Table

Register a group of functions as a module:

```rust
fn lib_greet(state: &mut LuaState) -> LuaResult<usize> {
    let name = state.get_arg(1)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "World".to_string());
    let result = state.create_string(&format!("Hello, {}!", name))?;
    state.push_value(result)?;
    Ok(1)
}

fn lib_version(state: &mut LuaState) -> LuaResult<usize> {
    state.push_value(LuaValue::integer(1))?;
    Ok(1)
}

// Create a module table
let module = vm.create_table(0, 2)?;
let k1 = vm.create_string("greet")?;
let k2 = vm.create_string("version")?;
vm.raw_set(&module, k1, LuaValue::cfunction(lib_greet));
vm.raw_set(&module, k2, LuaValue::cfunction(lib_version));
vm.set_global("mylib", module)?;

vm.execute(r#"
    print(mylib.greet("Lua"))  -- Hello, Lua!
    print(mylib.version())     -- 1
"#)?;
```

## Next

- [FromLua / IntoLua](05-FromLuaIntoLua.md) — automatic type conversion traits
- [Error Handling](06-ErrorHandling.md) — error types and protected calls
