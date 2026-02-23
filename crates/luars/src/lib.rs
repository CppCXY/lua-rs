// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

// Crate-level clippy allows for design choices
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::len_without_is_empty)]
#![allow(clippy::module_inception)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::enum_variant_names)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::same_item_push)]

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
pub use lua_value::UserDataBuilder;
pub use lua_value::userdata_trait::{
    LuaEnum, LuaMethodProvider, LuaRegistrable, LuaStaticMethodProvider, OpaqueUserData,
    RefUserData, UdValue, UserDataTrait,
};

#[cfg(test)]
use crate::lua_vm::SafeOption;
pub use gc::*;
pub use lib_registry::LibraryRegistry;
pub use lua_value::RustCallback;
pub use lua_value::lua_convert::{FromLua, IntoLua};
pub use lua_value::{Chunk, LuaFunction, LuaTable, LuaValue};
pub use lua_vm::async_thread::{AsyncCallHandle, AsyncFuture, AsyncReturnValue, AsyncThread};
pub use lua_vm::lua_error::LuaFullError;
pub use lua_vm::table_builder::TableBuilder;
pub use lua_vm::{
    Instruction, LuaAnyRef, LuaFunctionRef, LuaResult, LuaStringRef, LuaTableRef, LuaVM, OpCode,
};
pub use stdlib::Stdlib;
