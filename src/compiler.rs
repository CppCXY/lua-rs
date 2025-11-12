// Lua bytecode compiler
// Simple demo compiler that generates bytecode for testing

use crate::opcode::{OpCode, Instruction};
use crate::value::{Chunk, LuaValue};

pub struct Compiler {
    chunk: Chunk,
    next_register: u32,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            chunk: Chunk::new(),
            next_register: 0,
        }
    }

    /// Simplified compiler - for now, just generates a simple test program
    /// In a real implementation, this would parse the AST from emmylua_parser
    pub fn compile(_source: &str) -> Result<Chunk, String> {
        let mut compiler = Compiler::new();
        
        // For now, just create a simple test program
        // This is a placeholder until we properly integrate with emmylua_parser
        compiler.compile_simple()?;
        
        Ok(compiler.chunk)
    }
    
    /// Simple demo compilation - generates bytecode that returns a number
    fn compile_simple(&mut self) -> Result<(), String> {
        // Example: return 42
        
        // Load constant 42 into register 0
        let const_idx = self.add_constant(LuaValue::number(42.0));
        let reg0 = self.alloc_register();
        self.emit(Instruction::encode_abx(OpCode::LoadK, reg0, const_idx));
        
        // Return register 0
        self.emit(Instruction::encode_abc(OpCode::Return, reg0, 2, 0));
        
        Ok(())
    }

    // Helper methods
    fn emit(&mut self, instr: u32) -> usize {
        self.chunk.code.push(instr);
        self.chunk.code.len() - 1
    }

    fn add_constant(&mut self, value: LuaValue) -> u32 {
        self.chunk.constants.push(value);
        (self.chunk.constants.len() - 1) as u32
    }

    fn alloc_register(&mut self) -> u32 {
        let reg = self.next_register;
        self.next_register += 1;
        if self.next_register as usize > self.chunk.max_stack_size {
            self.chunk.max_stack_size = self.next_register as usize;
        }
        reg
    }
}
