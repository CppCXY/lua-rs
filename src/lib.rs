// Lua Runtime
// A compact Lua VM implementation with bytecode compiler and GC

pub mod value;
pub mod opcode;
pub mod compiler;
pub mod vm;
pub mod gc;

pub use value::{LuaValue, LuaString, LuaTable, LuaFunction, Chunk};
pub use opcode::{OpCode, Instruction};
pub use compiler::Compiler;
pub use vm::VM;
pub use gc::GC;

use std::rc::Rc;

/// Main entry point for executing Lua code
pub fn execute(source: &str) -> Result<LuaValue, String> {
    // Compile source to bytecode
    let chunk = Compiler::compile(source)?;
    
    // Debug: print chunk info
    eprintln!("=== DEBUG START ===");
    eprintln!("Constants count: {}", chunk.constants.len());
    for i in 0..chunk.constants.len() {
        let c = &chunk.constants[i];
        eprintln!("Constant[{}]: int={} float={}", i, c.is_integer(), c.is_float());
        if let Some(n) = c.as_integer() {
            eprintln!("  -> integer value: {}", n);
        }
        if let Some(f) = c.as_float() {
            eprintln!("  -> float value: {}", f);
        }
    }
    eprintln!("=== DEBUG END ===");
    if !chunk.code.is_empty() {
        eprintln!("DEBUG: Instructions:");
        for (i, instr) in chunk.code.iter().enumerate() {
            let opcode = Instruction::get_opcode(*instr);
            eprintln!("  [{}] {:?}", i, opcode);
        }
    }
    
    // Create VM and execute
    let mut vm = VM::new();
    let result = vm.execute(Rc::new(chunk))?;
    eprintln!("DEBUG: Result = {:?}", result);
    Ok(result)
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
