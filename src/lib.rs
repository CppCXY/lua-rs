// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

#[cfg(test)]
mod test;

pub mod compiler;
pub mod ffi;
pub mod gc;
pub mod lib_registry;
pub mod lua_pattern;
pub mod lua_value;
pub mod lua_vm;
pub mod object_pool;
pub mod stdlib;

pub use compiler::Compiler;
pub use ffi::FFIState;
pub use gc::GC;
pub use lib_registry::LibraryRegistry;
pub use lua_value::{Chunk, LuaFunction, LuaString, LuaTable, LuaValue};
pub use lua_vm::{LuaVM, Instruction, OpCode};
pub use object_pool::*;
use std::rc::Rc;

/// Main entry point for executing Lua code
pub fn execute(source: &str) -> Result<LuaValue, String> {
    // Create VM and compile using its string pool
    let mut vm = LuaVM::new();
    let chunk = vm.compile(source)?;

    vm.open_libs();
    vm.execute(Rc::new(chunk))
}

/// Execute Lua code with custom VM instance
pub fn execute_with_vm(vm: &mut LuaVM, source: &str) -> Result<LuaValue, String> {
    let chunk = vm.compile(source)?;
    vm.open_libs();
    vm.execute(Rc::new(chunk))
}
