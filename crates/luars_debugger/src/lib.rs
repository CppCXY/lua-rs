//! luars_debugger — Built-in EmmyLua-compatible debugger for luars.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use luars::LuaVM;
//! use luars_debugger::register_debugger;
//!
//! let mut vm = LuaVM::new();
//! // Simple: register with defaults (module = "emmy_core")
//! register_debugger(&mut vm).unwrap();
//! // Now Lua code can do: local dbg = require("emmy_core")
//! ```
//!
//! # Builder API
//!
//! ```rust,ignore
//! use luars::LuaVM;
//! use luars_debugger::DebuggerBuilder;
//!
//! let mut vm = LuaVM::new();
//! DebuggerBuilder::new()
//!     .module_name("debugger")           // custom module name
//!     .file_extensions(vec!["lua", "luau"]) // file types to match
//!     .register(&mut vm)
//!     .unwrap();
//! // Now Lua code can do: local dbg = require("debugger")
//! ```

pub mod debugger;
pub mod emmy_core;
pub mod hook_state;
pub mod proto;
pub mod transporter;

use debugger::Debugger;
use emmy_core::luaopen_emmy_core;

/// Builder for configuring and registering the debugger.
///
/// Provides a fluent API for debugger configuration before registration.
pub struct DebuggerBuilder {
    module_name: String,
    file_extensions: Vec<String>,
}

impl Default for DebuggerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DebuggerBuilder {
    /// Create a new builder with default settings.
    ///
    /// Defaults:
    /// - module_name: `"emmy_core"`
    /// - file_extensions: `["lua"]`
    pub fn new() -> Self {
        Self {
            module_name: "emmy_core".to_string(),
            file_extensions: vec!["lua".to_string()],
        }
    }

    /// Set the module name used for `require()`.
    ///
    /// Default: `"emmy_core"`.
    pub fn module_name(mut self, name: impl Into<String>) -> Self {
        self.module_name = name.into();
        self
    }

    /// Set file extensions the debugger should consider for source matching.
    ///
    /// Default: `["lua"]`.
    pub fn file_extensions(mut self, exts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.file_extensions = exts.into_iter().map(|e| e.into()).collect();
        self
    }

    /// Build and register the debugger with the given LuaVM.
    ///
    /// This creates the debugger instance, applies configuration, and injects
    /// the module into `package.preload` so Lua can load it via `require`.
    pub fn register(self, vm: &mut luars::LuaVM) -> luars::LuaResult<()> {
        let dbg = Debugger::new();
        {
            let mut s = dbg.state.lock().unwrap();
            s.file_extensions = self.file_extensions;
        }
        emmy_core::set_debugger(dbg);
        vm.register_preload(&self.module_name, luaopen_emmy_core)?;
        Ok(())
    }
}

/// Register the debugger with default settings.
///
/// Convenience function equivalent to `DebuggerBuilder::new().register(vm)`.
///
/// Injects `emmy_core` into `package.preload` so that Lua code
/// can load the debugger via `require "emmy_core"`.
pub fn register_debugger(vm: &mut luars::LuaVM) -> luars::LuaResult<()> {
    DebuggerBuilder::new().register(vm)
}
