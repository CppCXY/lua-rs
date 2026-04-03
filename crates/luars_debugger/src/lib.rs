//! luars_debugger — Built-in EmmyLua-compatible debugger for luars.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use luars::LuaVM;
//! use luars_debugger::Library;
//!
//! let mut vm = LuaVM::new();
//! // Simple: install with defaults (module = "emmy_core")
//! vm.install_library(Library::default()).unwrap();
//! // Now Lua code can do: local dbg = require("emmy_core")
//! ```
//!
//! # Customization
//!
//! ```rust,ignore
//! use luars::LuaVM;
//! use luars_debugger::Library;
//!
//! let mut vm = LuaVM::new();
//! vm.install_library(
//!     Library {
//!         module_name: "debugger".to_string(),
//!         ..Library::default()
//!     },
//! )
//! .unwrap();
//! // Now Lua code can do: local dbg = require("debugger")
//! ```

pub mod debugger;
pub mod emmy_core;
pub mod hook_state;
pub mod proto;
pub mod transporter;

use debugger::Debugger;
use emmy_core::luaopen_emmy_core;

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
    fn install_vm(&self, vm: &mut luars::LuaVM) -> luars::LuaResult<()> {
        let dbg = Debugger::new();
        {
            let mut s = dbg.state.lock().unwrap();
            s.file_extensions = self.file_extensions.clone();
        }
        emmy_core::set_debugger(dbg);
        vm.register_preload(&self.module_name, luaopen_emmy_core)?;
        Ok(())
    }
}
