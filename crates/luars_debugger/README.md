# luars_debugger

luars_debugger is a built-in debugger for luars, loaded via `require('emmy_core')`. It provides a DAP-compatible debugging interface, allowing IDEs to connect and debug Lua code running on luars. The debugger is implemented in Rust

## integrated with luars

add lib to `Cargo.toml`:

```toml
[dependencies]
luars_debugger = "0.14.0"
```

then add this line to your rust code:

```rust 
luars_debugger::register_debugger(&mut vm).unwrap();
```

The remaining steps are the same as for the EmmyLua debugger. See the EmmyLua debugger documentation: https://github.com/EmmyLua/EmmyLuaDebugger

If you need a DAP implementation, you can use: https://github.com/EmmyLuaLs/emmylua_dap