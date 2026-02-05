// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

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

#[cfg(test)]
use crate::lua_vm::SafeOption;
pub use gc::*;
pub use lib_registry::LibraryRegistry;
pub use lua_value::{Chunk, LuaFunction, LuaTable, LuaValue};
pub use lua_vm::{Instruction, LuaResult, LuaVM, OpCode};
pub use stdlib::Stdlib;
