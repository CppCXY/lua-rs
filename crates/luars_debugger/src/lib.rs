//! luars_debugger — Built-in EmmyLua-compatible debugger for luars.
//!
//! Usage:
//! ```rust,ignore
//! use luars::LuaVM;
//! use luars_debugger::register_debugger;
//!
//! let mut vm = LuaVM::new();
//! register_debugger(&mut vm).unwrap();
//! // Now Lua code can do: local dbg = require("emmy_core")
//! ```

pub mod debugger;
pub mod emmy_core;
pub mod hook_state;
pub mod proto;
pub mod transporter;

use debugger::Debugger;
use emmy_core::emmy_core_loader;

/// Register the debugger with a LuaVM instance.
///
/// This injects `emmy_core` into `package.preload` so that Lua code
/// can load the debugger via `require "emmy_core"`.
///
/// A global Debugger instance is created and stored for the hook callbacks.
pub fn register_debugger(vm: &mut luars::LuaVM) -> luars::LuaResult<()> {
    // Create and store the global debugger
    let dbg = Debugger::new();
    emmy_core::set_debugger(dbg);

    // Register the loader into package.preload["emmy_core"]
    vm.register_preload("emmy_core", emmy_core_loader)?;

    Ok(())
}
