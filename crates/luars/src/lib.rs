// Lua Runtime

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

mod compiler;
mod gc;
mod lib_registry;
mod lua_api;
mod lua_value;
mod lua_vm;
mod platform_time;
mod stdlib;

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

pub use gc::*;
pub use lib_registry::LibraryRegistry;
pub use lua_api::*;
pub use lua_value::RustCallback;
pub use lua_value::lua_convert::{FromLua, FromLuaMulti, IntoLua};
pub use lua_value::{LuaFunction, LuaProto, LuaTable, LuaValue, LuaValueKind, chunk_serializer::*};
pub use lua_vm::SafeOption;
#[cfg(feature = "sandbox")]
pub use lua_vm::SandboxConfig;
pub use lua_vm::async_thread::{
    AsyncCallHandle, AsyncFuture, AsyncReturnValue, AsyncThread, IntoAsyncLua,
};
pub use lua_vm::lua_error::{LuaError, LuaFullError};
pub use lua_vm::table_builder::TableBuilder;
pub use lua_vm::{
    CFunction, CallInfo, DebugInfo, Instruction, LuaAnyRef, LuaFunctionRef, LuaResult, LuaState,
    LuaStringRef, LuaTableRef, LuaVM, OpCode, UserDataRef,
};
pub use lua_vm::{LUA_MASKCALL, LUA_MASKCOUNT, LUA_MASKLINE, LUA_MASKRET};
pub use stdlib::Stdlib;
