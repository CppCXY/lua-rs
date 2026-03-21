pub mod call;
mod closure;
mod concat;
mod execute_loop;
pub(crate) mod helper;
mod hook;
pub(crate) mod metamethod;
mod number;
mod vararg;

pub use execute_loop::lua_execute;
pub use helper::{get_metamethod_event, get_metatable};
pub use metamethod::TmKind;
pub use metamethod::call_tm_res;
pub use metamethod::call_tm_res1;
