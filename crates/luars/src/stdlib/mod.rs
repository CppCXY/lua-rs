// Lua 5.4 Standard Libraries Implementation

pub mod basic;
pub mod coroutine;
pub mod debug;
// #[cfg(feature = "loadlib")]
// pub mod ffi;
pub mod io;
pub mod math;
pub mod os;
pub mod package;
mod sort_table;
pub mod string;
pub mod table;
pub mod utf8;

pub enum Stdlib {
    Io,
    Os,
    Math,
    String,
    Table,
    Basic,
    Package,
    Utf8,
    Coroutine,
    Debug,

    All,
}
