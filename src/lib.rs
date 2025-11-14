// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

pub mod compiler;
pub mod gc;
pub mod lib_registry;
pub mod lua_pattern;
pub mod lua_value;
pub mod opcode;
pub mod stdlib;
pub mod lua_vm;

pub use compiler::Compiler;
pub use gc::GC;
pub use lib_registry::LibraryRegistry;
pub use lua_value::{Chunk, LuaFunction, LuaString, LuaTable, LuaValue};
pub use opcode::{Instruction, OpCode};
use std::rc::Rc;
pub use lua_vm::LuaVM;

/// Main entry point for executing Lua code
pub fn execute(source: &str) -> Result<LuaValue, String> {
    // Compile source to bytecode
    let chunk = Compiler::compile(source)?;

    // Create VM and execute
    let mut vm = LuaVM::new();
    vm.execute(Rc::new(chunk))
}

/// Execute Lua code with custom VM instance
pub fn execute_with_vm(vm: &mut LuaVM, source: &str) -> Result<LuaValue, String> {
    let chunk = Compiler::compile(source)?;
    vm.execute(Rc::new(chunk))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_arithmetic() {
        let result = execute("local x = 10 + 20; return x");
        assert!(result.is_ok());
    }

    #[test]
    fn test_local_variables() {
        let result = execute("local x = 42; local y = x; return y");
        assert!(result.is_ok());
    }

    #[test]
    fn test_table_creation() {
        let result = execute("local t = {}; return t");
        assert!(result.is_ok());
    }

    #[test]
    fn test_boolean_logic() {
        let result = execute("local x = true; local y = not x; return y");
        assert!(result.is_ok());
    }
}
