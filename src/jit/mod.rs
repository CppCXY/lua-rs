// JIT Compiler for Lua using Cranelift
// This module provides Just-In-Time compilation of Lua bytecode to native machine code
// Redesigned for efficiency: uses fixed-layout values and method-JIT approach
pub mod jit_fastpath;
pub mod jit_pattern;
pub mod jit_value;
pub mod runtime;

use crate::opcode::{Instruction, OpCode};
use crate::value::Chunk;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use jit_value::JitValue;
use std::collections::HashMap;
use std::mem;

/// Generic JIT function for type-specialized code paths
/// Takes registers array and returns success flag (0 = bailout to interpreter)
pub type JitFunction = unsafe extern "C" fn(*mut JitValue, *const JitValue, i32) -> i32;

/// JIT compilation statistics
#[derive(Debug, Default, Clone, Copy)]
pub struct JitStats {
    pub compilations: usize,
    pub native_executions: usize,
    pub failed_compilations: usize,
}

/// JIT Compiler using Cranelift
pub struct JitCompiler {
    /// Cranelift JIT module
    module: JITModule,
    /// Compiled function cache (chunk_ptr -> function)
    compiled_cache: HashMap<usize, (FuncId, JitFunction)>,
    /// Statistics
    pub stats: JitStats,
}

impl JitCompiler {
    /// Create a new JIT compiler
    pub fn new() -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder
            .set("use_colocated_libcalls", "false")
            .map_err(|e| format!("Failed to set flag: {}", e))?;
        flag_builder
            .set("is_pic", "false")
            .map_err(|e| format!("Failed to set flag: {}", e))?;
        flag_builder
            .set("opt_level", "speed")
            .map_err(|e| format!("Failed to set flag: {}", e))?;

        let isa_builder = cranelift_native::builder()
            .map_err(|e| format!("Failed to create ISA builder: {}", e))?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| format!("Failed to create ISA: {}", e))?;

        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let module = JITModule::new(builder);

        Ok(JitCompiler {
            module,
            compiled_cache: HashMap::new(),
            stats: JitStats::default(),
        })
    }

    /// Compile an entire chunk
    /// This replaces hot-path compilation with full-chunk compilation
    pub fn compile_chunk(&mut self, chunk: &Chunk) -> Result<JitFunction, String> {
        self.stats.compilations += 1;

        // Check cache first
        let chunk_id = chunk as *const Chunk as usize;
        if let Some(&(_, func)) = self.compiled_cache.get(&chunk_id) {
            return Ok(func);
        }

        // Quick check: can we compile this chunk?
        if !self.can_compile_chunk(chunk) {
            self.stats.failed_compilations += 1;
            return Err("Chunk contains unsupported features".to_string());
        }

        // Create function signature
        // fn(registers: *mut JitValue, constants: *const JitValue, pc: i32) -> i32
        let pointer_type = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(pointer_type)); // registers
        sig.params.push(AbiParam::new(pointer_type)); // constants
        sig.params.push(AbiParam::new(types::I32)); // initial pc
        sig.returns.push(AbiParam::new(types::I32)); // return pc

        // Declare function
        let func_name = format!("jit_chunk_{}", chunk_id);
        let func_id = self
            .module
            .declare_function(&func_name, Linkage::Local, &sig)
            .map_err(|e| format!("Failed to declare function: {}", e))?;

        // Create function builder
        let mut func = codegen::ir::Function::with_name_signature(
            codegen::ir::UserFuncName::user(0, func_id.as_u32()),
            sig,
        );

        let mut builder_context = FunctionBuilderContext::new();
        let mut func_builder = FunctionBuilder::new(&mut func, &mut builder_context);

        // Create entry block
        let entry_block = func_builder.create_block();
        func_builder.append_block_params_for_function_params(entry_block);
        func_builder.switch_to_block(entry_block);

        // Get function parameters
        let registers_ptr = func_builder.block_params(entry_block)[0];
        let constants_ptr = func_builder.block_params(entry_block)[1];
        let _initial_pc = func_builder.block_params(entry_block)[2];

        // Compile all instructions
        let value_size = JitValue::SIZE as i32;

        // Create blocks for each instruction (for jump targets)
        let mut blocks: Vec<Block> = Vec::new();
        for _ in 0..chunk.code.len() {
            blocks.push(func_builder.create_block());
        }

        // Create exit block (for return/unsupported operations)
        let exit_block = func_builder.create_block();
        func_builder.append_block_params_for_function_returns(exit_block);

        // Jump to initial PC
        func_builder.seal_block(entry_block);

        for (pc, &instr) in chunk.code.iter().enumerate() {
            func_builder.switch_to_block(blocks[pc]);

            let opcode = Instruction::get_opcode(instr);

            match opcode {
                OpCode::Move
                | OpCode::LoadK
                | OpCode::LoadNil
                | OpCode::LoadBool
                | OpCode::Add
                | OpCode::Sub
                | OpCode::Mul => {
                    // Compile this instruction
                    Self::compile_instruction(
                        &mut func_builder,
                        instr,
                        opcode,
                        registers_ptr,
                        constants_ptr,
                        pointer_type,
                        value_size,
                    )?;

                    // Jump to next instruction
                    if pc + 1 < chunk.code.len() {
                        func_builder.ins().jump(blocks[pc + 1], &[]);
                    } else {
                        // End of chunk
                        let ret_pc = func_builder.ins().iconst(types::I32, (pc + 1) as i64);
                        func_builder.ins().return_(&[ret_pc]);
                    }
                }

                OpCode::Jmp => {
                    let sbx = Instruction::get_sbx(instr);
                    let target_pc = (pc as i32 + 1 + sbx) as usize;

                    if target_pc < chunk.code.len() {
                        func_builder.ins().jump(blocks[target_pc], &[]);
                    } else {
                        let ret_pc = func_builder.ins().iconst(types::I32, target_pc as i64);
                        func_builder.ins().return_(&[ret_pc]);
                    }
                }

                OpCode::Return | _ => {
                    // Unsupported operation: return to interpreter
                    let ret_pc = func_builder.ins().iconst(types::I32, pc as i64);
                    func_builder.ins().return_(&[ret_pc]);
                }
            }

            func_builder.seal_block(blocks[pc]);
        }

        // Finalize function
        func_builder.finalize();

        // Compile to machine code
        let mut ctx = codegen::Context::for_function(func);
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define function: {}", e))?;

        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize: {}", e))?;

        // Get function pointer
        let code_ptr = self.module.get_finalized_function(func_id);
        let jit_func: JitFunction = unsafe { mem::transmute(code_ptr) };

        // Cache the compiled function
        self.compiled_cache.insert(chunk_id, (func_id, jit_func));

        Ok(jit_func)
    }


    /// Check if a chunk can be compiled
    fn can_compile_chunk(&self, chunk: &Chunk) -> bool {
        for &instr in &chunk.code {
            let opcode = Instruction::get_opcode(instr);
            match opcode {
                OpCode::Move
                | OpCode::LoadK
                | OpCode::LoadNil
                | OpCode::LoadBool
                | OpCode::Add
                | OpCode::Sub
                | OpCode::Mul
                | OpCode::Jmp => {
                    // Supported
                }
                _ => {
                    // Unsupported
                    return false;
                }
            }
        }
        true
    }

    /// Compile a single instruction to Cranelift IR
    fn compile_instruction(
        builder: &mut FunctionBuilder,
        instr: u32,
        opcode: OpCode,
        registers_ptr: Value,
        constants_ptr: Value,
        pointer_type: Type,
        value_size: i32,
    ) -> Result<(), String> {
        match opcode {
            OpCode::Move => {
                let a = Instruction::get_a(instr) as i32;
                let b = Instruction::get_b(instr) as i32;

                // Load R(B)
                let b_offset = builder.ins().iconst(pointer_type, (b * value_size) as i64);
                let b_addr = builder.ins().iadd(registers_ptr, b_offset);

                // Load both tag and data
                let tag = builder.ins().load(types::I64, MemFlags::new(), b_addr, 0);
                let data = builder.ins().load(types::I64, MemFlags::new(), b_addr, 8);

                // Store to R(A)
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                builder.ins().store(MemFlags::new(), tag, a_addr, 0);
                builder.ins().store(MemFlags::new(), data, a_addr, 8);
            }

            OpCode::LoadK => {
                let a = Instruction::get_a(instr) as i32;
                let bx = Instruction::get_bx(instr) as i32;

                // Load K(Bx)
                let k_offset = builder.ins().iconst(pointer_type, (bx * value_size) as i64);
                let k_addr = builder.ins().iadd(constants_ptr, k_offset);
                let tag = builder.ins().load(types::I64, MemFlags::new(), k_addr, 0);
                let data = builder.ins().load(types::I64, MemFlags::new(), k_addr, 8);

                // Store to R(A)
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                builder.ins().store(MemFlags::new(), tag, a_addr, 0);
                builder.ins().store(MemFlags::new(), data, a_addr, 8);
            }

            OpCode::Add => {
                // For now, we'll handle simple integer addition
                // More complex type checking would be done at runtime
                let a = Instruction::get_a(instr) as i32;
                let b = Instruction::get_b(instr) as i32;
                let c = Instruction::get_c(instr) as i32;

                // Load R(B) - assume it's an integer for JIT compilation
                let b_offset = builder.ins().iconst(pointer_type, (b * value_size) as i64);
                let b_addr = builder.ins().iadd(registers_ptr, b_offset);
                // Skip the discriminant (first 8 bytes) and load the i64 value
                let b_val = builder.ins().load(types::I64, MemFlags::new(), b_addr, 8);

                // Load R(C)
                let c_offset = builder.ins().iconst(pointer_type, (c * value_size) as i64);
                let c_addr = builder.ins().iadd(registers_ptr, c_offset);
                let c_val = builder.ins().load(types::I64, MemFlags::new(), c_addr, 8);

                // Add
                let result = builder.ins().iadd(b_val, c_val);

                // Store result (tag = 2 for Integer in JitValue)
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                let int_tag = builder.ins().iconst(types::I64, 2);
                builder.ins().store(MemFlags::new(), int_tag, a_addr, 0);
                builder.ins().store(MemFlags::new(), result, a_addr, 8);
            }

            OpCode::Sub => {
                let a = Instruction::get_a(instr) as i32;
                let b = Instruction::get_b(instr) as i32;
                let c = Instruction::get_c(instr) as i32;

                let b_offset = builder.ins().iconst(pointer_type, (b * value_size) as i64);
                let b_addr = builder.ins().iadd(registers_ptr, b_offset);
                let b_val = builder.ins().load(types::I64, MemFlags::new(), b_addr, 8);

                let c_offset = builder.ins().iconst(pointer_type, (c * value_size) as i64);
                let c_addr = builder.ins().iadd(registers_ptr, c_offset);
                let c_val = builder.ins().load(types::I64, MemFlags::new(), c_addr, 8);

                let result = builder.ins().isub(b_val, c_val);

                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                let discriminant = builder.ins().iconst(types::I64, 3);
                builder
                    .ins()
                    .store(MemFlags::new(), discriminant, a_addr, 0);
                builder.ins().store(MemFlags::new(), result, a_addr, 8);
            }

            OpCode::Mul => {
                let a = Instruction::get_a(instr) as i32;
                let b = Instruction::get_b(instr) as i32;
                let c = Instruction::get_c(instr) as i32;

                let b_offset = builder.ins().iconst(pointer_type, (b * value_size) as i64);
                let b_addr = builder.ins().iadd(registers_ptr, b_offset);
                let b_val = builder.ins().load(types::I64, MemFlags::new(), b_addr, 8);

                let c_offset = builder.ins().iconst(pointer_type, (c * value_size) as i64);
                let c_addr = builder.ins().iadd(registers_ptr, c_offset);
                let c_val = builder.ins().load(types::I64, MemFlags::new(), c_addr, 8);

                let result = builder.ins().imul(b_val, c_val);

                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                let discriminant = builder.ins().iconst(types::I64, 3);
                builder
                    .ins()
                    .store(MemFlags::new(), discriminant, a_addr, 0);
                builder.ins().store(MemFlags::new(), result, a_addr, 8);
            }

            OpCode::LoadNil => {
                let a = Instruction::get_a(instr) as i32;
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);

                // Nil tag = 0
                let tag = builder.ins().iconst(types::I64, 0);
                let data = builder.ins().iconst(types::I64, 0);
                builder.ins().store(MemFlags::new(), tag, a_addr, 0);
                builder.ins().store(MemFlags::new(), data, a_addr, 8);
            }

            OpCode::LoadBool => {
                let a = Instruction::get_a(instr) as i32;
                let b = Instruction::get_b(instr);

                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);

                // Boolean tag = 1
                let tag = builder.ins().iconst(types::I64, 1);
                let data = builder.ins().iconst(types::I64, if b != 0 { 1 } else { 0 });
                builder.ins().store(MemFlags::new(), tag, a_addr, 0);
                builder.ins().store(MemFlags::new(), data, a_addr, 8);
            }

            _ => {
                // For complex operations, we would need to call back to the interpreter
                // or implement more sophisticated compilation
                return Err(format!("Opcode {:?} not yet supported in JIT", opcode));
            }
        }

        Ok(())
    }

    /// Get a compiled function from cache
    pub fn get_compiled(&self, chunk_id: usize) -> Option<JitFunction> {
        self.compiled_cache.get(&chunk_id).map(|&(_, f)| f)
    }

    /// Clear the compilation cache
    pub fn clear_cache(&mut self) {
        self.compiled_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_creation() {
        let jit = JitCompiler::new();
        assert!(jit.is_ok());
    }
}
