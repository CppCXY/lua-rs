# luars_debugger

luars_debugger is an EmmyLua-compatible debugger module for luars, loaded via `require("emmy_core")`. It provides a DAP-compatible debugging interface, allowing IDEs to connect and debug Lua code running on luars.

The recommended integration path now uses luars's unified external-library API.

## Install

Add the crate to `Cargo.toml`:

```toml
[dependencies]
luars = "0.18"
luars_debugger = "0.18"
```

Then install the debugger into your runtime:

```rust
use luars::{LuaVM, SafeOption};
use luars_debugger::Library;

let mut vm = LuaVM::new(SafeOption::default());
vm.install_library(Library::default())?;

// Lua can now load the debugger with require("emmy_core")
```

If you use the high-level API, the same installation flow is available there too:

```rust
use luars::{Lua, SafeOption};
use luars_debugger::Library;

let mut lua = Lua::new(SafeOption::default());
lua.install_library(Library::default())?;
```

## Custom module name

```rust
use luars::{LuaVM, SafeOption};
use luars_debugger::Library;

let mut vm = LuaVM::new(SafeOption::default());
vm.install_library(
	Library {
		module_name: "debugger".to_string(),
		..Library::default()
	},
)?;

// Lua can now load the debugger with require("debugger")
```

The remaining steps are the same as for the EmmyLua debugger. See the EmmyLua debugger documentation: https://github.com/EmmyLua/EmmyLuaDebugger

If you need a DAP implementation, you can use: https://github.com/EmmyLuaLs/emmylua_dap