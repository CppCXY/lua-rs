# luars

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Crates.io](https://img.shields.io/crates/v/luars.svg)](https://crates.io/crates/luars)

A Lua 5.5 runtime implementation in Rust, providing both interpreter and embedding capabilities.

## Features

- **Lua 5.5 Core**: Implements Lua 5.5 language semantics
- **Pure Rust**: Written entirely in Rust with no C dependencies
- **Embeddable**: Designed to be embedded in Rust applications
- **Standard Libraries**: Comprehensive coverage of Lua standard libraries
- **UTF-8 Strings**: All Lua strings are valid UTF-8 (non-standard but safer)
- **UserData API**: Derive macros to expose Rust structs to Lua with fields, methods, and constructors
- **Rust Closures**: Register Rust closures (with captured state) as Lua functions via `RClosure`
- **FromLua / IntoLua**: Automatic type conversion traits for seamless Rust ↔ Lua interop
- **Optional Serde**: JSON serialization support for Lua values (feature-gated)

## Current Status

### ✅ Implemented
- Core language features (operators, control flow, functions, tables)
- Metatables and metamethods
- Coroutines
- Garbage collection
- Most standard libraries: `string`, `table`, `math`, `io`, `os`, `coroutine`, `utf8`, `package`
- String pack/unpack with Lua 5.4+ formats (`i[n]`, `I[n]`, `j`, `n`, `T`)
- Lua reference mechanism (luaL_ref/luaL_unref)
- Optional JSON serialization via serde feature

### ⚠️ Known Limitations
- **UTF-8 Only Strings**: Unlike standard Lua, strings must be valid UTF-8. Binary data should use the binary type from `string.pack`/`string.unpack`.
- **Custom Bytecode Format**: Uses LuaRS-specific bytecode format, not compatible with standard Lua bytecode.
- **Partial Debug Library**: Some introspection features are not yet implemented.

### 🚧 Incomplete Features
- Full debug library introspection

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
luars = "0.4"

# Optional: Enable JSON serialization support
luars = { version = "0.4", features = ["serde"] }
```

### Basic Example

```rust
use luars::lua_vm::{LuaVM, SafeOption};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(luars::Stdlib::All).unwrap();
    let result = vm.execute_string(r#"
        function greet(name)
            return "Hello, " .. name .. "!"
        end
        
        print(greet("World"))
    "#);
    
    Ok(())
}
```

### UserData Example

```rust
use luars::{LuaUserData, lua_methods};

#[derive(LuaUserData)]
#[lua_impl(Display)]
struct Point { pub x: f64, pub y: f64 }

#[lua_methods]
impl Point {
    pub fn new(x: f64, y: f64) -> Self { Point { x, y } }
    pub fn distance(&self) -> f64 { (self.x * self.x + self.y * self.y).sqrt() }
}

// Register and use:
let state = vm.main_state();
state.register_type_of::<Point>("Point")?;
state.execute_string(r#"
    local p = Point.new(3, 4)
    print(p.x, p:distance())   -- 3.0  5.0
"#)?;
```

### Rust Closures in Lua

```rust
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

let counter = Arc::new(AtomicUsize::new(0));
let counter_clone = counter.clone();
let func = vm.create_closure(move |state| {
    let n = counter_clone.fetch_add(1, Ordering::SeqCst);
    state.push_value(luars::LuaValue::integer(n as i64))?;
    Ok(1)
})?;
vm.set_global("next_id", func)?;
```

## Standard Library Coverage

| Library | Status | Notes |
|---------|--------|-------|
| `basic` | ✅ Complete | All core functions implemented |
| `string` | ✅ Complete | UTF-8 strings only |
| `table` | ✅ Complete | All table manipulation functions |
| `math` | ✅ Complete | Full math library including random |
| `io` | ✅ Complete | File I/O operations |
| `os` | ✅ Complete | Operating system facilities |
| `coroutine` | ✅ Complete | Full coroutine support |
| `utf8` | ✅ Complete | UTF-8 string operations |
| `package` | ✅ Complete | Module system with require |
| `debug` | ⚠️ Partial | Some introspection features missing |

## Differences from Standard Lua

1. **UTF-8 Strings**: All Lua strings must be valid UTF-8. The `\xNN` escape sequences that would produce invalid UTF-8 are not supported. Use `string.pack()` for binary data.

2. **Bytecode Format**: Uses a custom LuaRS bytecode format. `string.dump()` output is not compatible with standard Lua.

3. **String/Binary Comparison**: String and binary values can be compared for equality, unlike standard Lua where they would be different types.

## Building from Source

```bash
# Clone the repository
git clone https://github.com/CppCXY/lua-rs
cd lua-rs

# Build the library
cargo build --release

# Run tests
cargo test

# Build with serde support
cargo build --release --features serde
```

## Contributing

Contributions are welcome! This project is actively developed and we appreciate:

- Bug reports and fixes
- Performance improvements
- Documentation improvements
- Test cases for edge cases
- Feature implementations

Please open an issue before starting major work to discuss the approach.

## Roadmap

- [ ] Complete debug library implementation
- [ ] Performance optimizations

## Documentation

See [docs/Guide.md](../../docs/Guide.md) for the comprehensive usage guide covering:
- VM creation and configuration
- Executing Lua code
- Working with values, globals, and tables
- Registering Rust functions and closures
- UserData (exposing Rust structs to Lua)
- FromLua / IntoLua type conversion
- Error handling
- Full API reference


## License

MIT License - See [LICENSE](../../LICENSE) file for details.
