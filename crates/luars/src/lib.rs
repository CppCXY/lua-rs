// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

// Allow the derive macro to use `luars::...` paths even inside this crate
extern crate self as luars;

#[cfg(test)]
mod test;

pub mod compiler;
pub mod gc;
pub mod lib_registry;
pub mod lua_value;
pub mod lua_vm;
pub mod stdlib;

#[cfg(feature = "serde")]
pub mod serde;

// Re-export the derive macros so users can `use luars::LuaUserData;`
pub use luars_derive::LuaUserData;
pub use luars_derive::lua_methods;

// Re-export userdata trait types at crate root for convenience
pub use lua_value::LuaUserdata;
pub use lua_value::userdata_trait::{LuaMethodProvider, UdValue, UserDataTrait};

#[cfg(test)]
use crate::lua_vm::SafeOption;
pub use gc::*;
pub use lib_registry::LibraryRegistry;
pub use lua_value::{Chunk, LuaFunction, LuaTable, LuaValue};
pub use lua_vm::{Instruction, LuaResult, LuaVM, OpCode};
pub use stdlib::Stdlib;
