//! luars_debugger — Built-in EmmyLua-compatible debugger for luars.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use luars::{Lua, SafeOption, Stdlib};
//! use luars_debugger::Library;
//!
//! let mut lua = Lua::new(SafeOption::default());
//! lua.open_stdlib(Stdlib::All).unwrap();
//! lua.install_library(Library::default()).unwrap();
//!
//! // The debugger table is available both globally and through require().
//! lua.execute("local dbg = require('emmy_core'); assert(type(dbg.breakHere) == 'function')")
//!     .unwrap();
//! ```
//!
//! # Customization
//!
//! ```rust,ignore
//! use luars::{Lua, SafeOption, Stdlib};
//! use luars_debugger::Library;
//!
//! let mut lua = Lua::new(SafeOption::default());
//! lua.open_stdlib(Stdlib::All).unwrap();
//! lua.install_library(
//!     Library {
//!         module_name: "debugger".to_string(),
//!         ..Library::default()
//!     },
//! )
//! .unwrap();
//! // Now Lua code can do: local dbg = require("debugger")
//! ```
//!
//! The debugger itself still uses raw `LuaState` callbacks internally because
//! hook installation and stepping must touch VM internals, but embedders only
//! interact with the high-level `Lua` installation surface. Internally the
//! exported module is described with `luars::lua_module!`, which is the intended
//! user-facing registration pattern.

pub mod debugger;
mod emmy_core;
pub mod hook_state;
pub mod proto;
pub mod transporter;

use debugger::Debugger;

/// Installable debugger library descriptor for luars.
#[derive(Clone, Debug)]
pub struct Library {
    pub module_name: String,
    pub file_extensions: Vec<String>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            module_name: "emmy_core".to_string(),
            file_extensions: vec!["lua".to_string()],
        }
    }
}

impl luars::LuaLibrary for Library {
    fn install(&self, lua: &mut luars::Lua) -> luars::LuaResult<()> {
        let dbg = Debugger::new();
        {
            let mut s = dbg.state.lock().unwrap();
            s.file_extensions = self.file_extensions.clone();
        }
        emmy_core::set_debugger(dbg);
        lua.install_library(luars::lua_module!(self.module_name.clone(), {
            "tcpListen" => emmy_core::emmy_tcp_listen,
            "tcpConnect" => emmy_core::emmy_tcp_connect,
            "waitIDE" => emmy_core::emmy_wait_ide,
            "breakHere" => emmy_core::emmy_break_here,
            "stop" => emmy_core::emmy_stop,
        }))
    }
}

#[cfg(test)]
mod tests {
    use luars::LuaApi;

    use super::Library;

    #[test]
    fn installs_debugger_module_for_require() {
        let mut lua = luars::Lua::new(luars::SafeOption::default());
        lua.open_stdlib(luars::Stdlib::All).unwrap();
        lua.install_library(Library::default()).unwrap();

        let (tcp_listen_ty, wait_ide_ty): (String, String) = lua
            .load("local dbg = require('emmy_core'); return type(dbg.tcpListen), type(dbg.waitIDE)")
            .eval_multi()
            .unwrap();

        assert_eq!(tcp_listen_ty, "function");
        assert_eq!(wait_ide_ty, "function");
    }
}
