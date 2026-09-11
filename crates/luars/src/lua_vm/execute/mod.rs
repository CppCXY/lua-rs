// execute --- Lua 5.5 VM 执行引擎。
//
// 唯一后端：lua_execute 位于 core.rs；各慢路径模块独立维护。
// 详细设计见 docs/execute.md。

pub(crate) mod arith;
pub mod call;
mod closure;
mod concat;
pub(crate) mod core;
pub(crate) mod helper;
mod hook;
pub(crate) mod metamethod;
mod number;
mod table_ops;
mod vararg;

pub use core::lua_execute;
pub use helper::{get_metamethod_event, get_metatable};
pub use metamethod::TmKind;
pub use metamethod::call_tm_res;
pub use metamethod::call_tm_res1;
