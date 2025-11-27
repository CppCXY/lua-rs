// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

#[cfg(test)]
mod test;

pub mod compiler;
#[cfg(feature = "loadlib")]
pub mod ffi;
pub mod gc;
pub mod lib_registry;
#[cfg(feature = "async")]
pub mod lua_async;
pub mod lua_pattern;
pub mod lua_value;
pub mod lua_vm;
pub mod stdlib;
pub use compiler::Compiler;
#[cfg(feature = "loadlib")]
pub use ffi::FFIState;
pub use gc::*;
pub use lib_registry::LibraryRegistry;
pub use lua_value::{Chunk, LuaFunction, LuaString, LuaTable, LuaValue};
pub use lua_vm::{Instruction, LuaResult, LuaVM, OpCode};
use std::rc::Rc;

/// Main entry point for executing Lua code
pub fn execute(source: &str) -> LuaResult<LuaValue> {
    // Create VM and compile using its string pool
    let mut vm = LuaVM::new();
    vm.open_libs();
    let chunk = vm.compile(source)?;
    vm.execute(Rc::new(chunk))
}

/// Execute Lua code with custom VM instance
pub fn execute_with_vm(vm: &mut LuaVM, source: &str) -> LuaResult<LuaValue> {
    let chunk = vm.compile(source)?;
    vm.open_libs();
    vm.execute(Rc::new(chunk))
}
