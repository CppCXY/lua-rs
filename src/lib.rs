// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

pub mod value;
pub mod opcode;
pub mod compiler;
pub mod vm;
pub mod gc;
pub mod lib_registry;
pub mod stdlib;
pub mod lua_pattern;
pub mod jit;
pub mod jit_value;
pub mod jit_pattern;
pub mod jit_fastpath;

pub use value::{LuaValue, LuaString, LuaTable, LuaFunction, Chunk};
pub use opcode::{OpCode, Instruction};
pub use compiler::Compiler;
pub use vm::VM;
pub use gc::GC;
pub use lib_registry::LibraryRegistry;
pub use jit::JitCompiler;

// Re-export for public API
pub use jit_fastpath as fastpath;

use std::rc::Rc;

/// Main entry point for executing Lua code
pub fn execute(source: &str) -> Result<LuaValue, String> {
    // Compile source to bytecode
    let chunk = Compiler::compile(source)?;
    
    // Create VM and execute
    let mut vm = VM::new();
    vm.execute(Rc::new(chunk))
}

/// Execute Lua code with custom VM instance
pub fn execute_with_vm(vm: &mut VM, source: &str) -> Result<LuaValue, String> {
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
