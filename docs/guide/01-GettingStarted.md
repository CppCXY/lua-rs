# Getting Started

This guide walks you through creating a Lua VM, loading standard libraries, and running your first Lua script from Rust.

## Creating a VM

The entry point is `LuaVM::new()`, which takes a `SafeOption` configuration:

```rust
use luars::lua_vm::{LuaVM, SafeOption};

let mut vm = LuaVM::new(SafeOption::default());
```

### SafeOption

`SafeOption` controls resource limits for the VM:

```rust
pub struct SafeOption {
    pub max_call_depth: usize,       // default: 200
    pub max_stack_size: usize,       // default: 1_000_000
    pub max_gc_memory: usize,        // default: 512 * 1024 * 1024 (512 MB)
    pub max_instruction_count: usize, // default: 0 (unlimited)
}
```

Use `SafeOption::default()` for normal usage, or customize limits for sandboxed environments:

```rust
let option = SafeOption {
    max_call_depth: 100,
    max_stack_size: 10_000,
    max_gc_memory: 16 * 1024 * 1024,     // 16 MB
    max_instruction_count: 1_000_000,     // limit instructions
};
let mut vm = LuaVM::new(option);
```

## Loading Standard Libraries

Before executing Lua code, load the standard libraries you need:

```rust
use luars::Stdlib;

// Load all standard libraries
vm.open_stdlib(Stdlib::All)?;
```

### Available Libraries

| `Stdlib` variant | Lua globals | Description |
|-----------------|-------------|-------------|
| `Stdlib::Basic` | `print`, `type`, `tostring`, `tonumber`, `pcall`, `error`, ... | Core functions |
| `Stdlib::String` | `string.*` | String manipulation |
| `Stdlib::Table` | `table.*` | Table manipulation |
| `Stdlib::Math` | `math.*` | Math functions |
| `Stdlib::IO` | `io.*` | File I/O |
| `Stdlib::OS` | `os.*` | OS facilities |
| `Stdlib::Coroutine` | `coroutine.*` | Coroutine support |
| `Stdlib::Utf8` | `utf8.*` | UTF-8 operations |
| `Stdlib::Package` | `require`, `package.*` | Module system |
| `Stdlib::Debug` | `debug.*` | Debug library (partial) |
| `Stdlib::All` | All of the above | Load everything |

You can load libraries selectively:

```rust
vm.open_stdlib(Stdlib::Basic)?;
vm.open_stdlib(Stdlib::String)?;
vm.open_stdlib(Stdlib::Math)?;
```

## Running Lua Code

The simplest way to run Lua code:

```rust
let results = vm.execute_string(r#"
    print("Hello from Lua!")
    return 42
"#)?;

// results is Vec<LuaValue>
println!("{:?}", results[0].as_integer()); // Some(42)
```

## LuaVM vs LuaState

luars has two main types for interacting with Lua:

| Type | Access | Use case |
|------|--------|----------|
| `LuaVM` | Owns the entire VM (GC, allocator, threads) | Top-level operations: execute code, create objects, manage globals |
| `LuaState` | A single thread / execution context | Operations within a C/Rust function callback: access arguments, push results |

Get the main `LuaState` from a `LuaVM`:

```rust
let state: &mut LuaState = vm.main_state();
```

Both `LuaVM` and `LuaState` provide overlapping APIs for convenience. For example, both have `execute_string`, `set_global`, `create_table`, etc. Use whichever you have access to.

## Complete Minimal Example

```rust
use luars::lua_vm::{LuaVM, SafeOption};
use luars::Stdlib;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a VM with default settings
    let mut vm = LuaVM::new(SafeOption::default());

    // Load basic + string + math
    vm.open_stdlib(Stdlib::Basic)?;
    vm.open_stdlib(Stdlib::String)?;
    vm.open_stdlib(Stdlib::Math)?;

    // Run Lua code and get results
    let results = vm.execute_string(r#"
        local function factorial(n)
            if n <= 1 then return 1 end
            return n * factorial(n - 1)
        end
        return factorial(10)
    "#)?;

    println!("10! = {}", results[0].as_integer().unwrap()); // 3628800
    Ok(())
}
```

## Next

- [Executing Code](02-ExecutingCode.md) — compilation, chunk execution, return values
- [Working with Values](03-WorkingWithValues.md) — `LuaValue`, globals, tables
