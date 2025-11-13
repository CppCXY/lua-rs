// JIT Compiler for Lua using Cranelift
// This module provides Just-In-Time compilation of Lua bytecode to native machine code

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;
use std::mem;

use crate::opcode::{Instruction, OpCode};
use crate::value::{Chunk, LuaValue};

/// Compiled native function signature
/// Takes registers pointer and returns nothing (modifies registers in-place)
pub type JitFunction = unsafe extern "C" fn(*mut LuaValue, *const LuaValue) -> i32;

/// JIT compilation statistics
#[derive(Debug, Default)]
pub struct JitStats {
    pub compilations: usize,
    pub execution_count: usize,
    pub native_executions: usize,
}

/// Hot path detection threshold
const HOT_THRESHOLD: usize = 10;

/// Tracks execution counts for hot path detection
#[derive(Debug)]
pub struct HotPathTracker {
    counts: HashMap<usize, usize>, // pc -> execution count
    failed: std::collections::HashSet<usize>, // PCs where compilation failed
}

impl HotPathTracker {
    pub fn new() -> Self {
        HotPathTracker {
            counts: HashMap::new(),
            failed: std::collections::HashSet::new(),
        }
    }

    /// Record an execution at a given program counter
    /// Returns true if this location just became hot (first time hitting threshold)
    pub fn record(&mut self, pc: usize) -> bool {
        // Don't track locations that failed compilation
        if self.failed.contains(&pc) {
            return false;
        }
        
        let count = self.counts.entry(pc).or_insert(0);
        *count += 1;
        
        // Return true only when we JUST hit the threshold
        *count == HOT_THRESHOLD
    }

    /// Check if a location is hot
    pub fn is_hot(&self, pc: usize) -> bool {
        self.counts.get(&pc).map_or(false, |&c| c >= HOT_THRESHOLD)
    }

    /// Mark this location as compilation-failed (don't try again)
    pub fn mark_failed(&mut self, pc: usize) {
        self.failed.insert(pc);
        self.counts.remove(&pc);
    }

    /// Reset counter for a location (old reset method, now calls mark_failed)
    pub fn reset(&mut self, pc: usize) {
        self.mark_failed(pc);
    }
}

/// JIT Compiler using Cranelift
pub struct JitCompiler {
    /// Cranelift JIT module
    module: JITModule,
    /// Compiled function cache (chunk_id + pc -> function)
    compiled_cache: HashMap<(usize, usize), (FuncId, JitFunction)>,
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

    /// Compile a hot path starting from the given PC
    pub fn compile_hot_path(
        &mut self,
        chunk: &Chunk,
        start_pc: usize,
    ) -> Result<JitFunction, String> {
        self.stats.compilations += 1;

        // Check cache first
        let chunk_id = chunk as *const Chunk as usize;
        if let Some(&(_, func)) = self.compiled_cache.get(&(chunk_id, start_pc)) {
            return Ok(func);
        }

        // Create function signature
        // fn(registers: *mut LuaValue, constants: *const LuaValue) -> i32
        let pointer_type = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(pointer_type)); // registers
        sig.params.push(AbiParam::new(pointer_type)); // constants
        sig.returns.push(AbiParam::new(types::I32)); // return pc

        // Declare function
        let func_name = format!("jit_func_{}_{}", chunk_id, start_pc);
        let func_id = self
            .module
            .declare_function(&func_name, Linkage::Local, &sig)
            .map_err(|e| format!("Failed to declare function: {}", e))?;

        // Create function builder with fresh context
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
        func_builder.seal_block(entry_block);

        // Get function parameters
        let registers_ptr = func_builder.block_params(entry_block)[0];
        let constants_ptr = func_builder.block_params(entry_block)[1];

        // Compile instructions
        let mut pc = start_pc;
        let mut compiled_instructions = 0;
        const MAX_COMPILE_SIZE: usize = 200; // Limit compilation size

        // Get pointer type for later use
        let pointer_type = self.module.target_config().pointer_type();
        let value_size = mem::size_of::<LuaValue>() as i32;

        // Track blocks for control flow (for jumps)
        let mut block_map: HashMap<usize, Block> = HashMap::new();
        let mut sealed_blocks: std::collections::HashSet<Block> = std::collections::HashSet::new();
        block_map.insert(start_pc, entry_block);
        sealed_blocks.insert(entry_block);

        while pc < chunk.code.len() && compiled_instructions < MAX_COMPILE_SIZE {
            let instr = chunk.code[pc];
            let opcode = Instruction::get_opcode(instr);

            // Check if we should stop compilation
            match opcode {
                OpCode::Call | OpCode::Return => {
                    // Stop at function calls and returns for now
                    break;
                }
                OpCode::Jmp => {
                    // Handle jump instruction
                    let sbx = Instruction::get_sbx(instr);
                    let target_pc = (pc as i32 + 1 + sbx) as usize;
                    
                    // For backward jumps (loops), stop compilation to avoid complexity
                    if target_pc <= start_pc {
                        // Return current PC
                        let return_val = func_builder.ins().iconst(types::I32, (pc + 1) as i64);
                        func_builder.ins().return_(&[return_val]);
                        pc = chunk.code.len(); // Exit loop
                        break;
                    }
                    
                    // Create target block if it doesn't exist
                    if !block_map.contains_key(&target_pc) {
                        let target_block = func_builder.create_block();
                        block_map.insert(target_pc, target_block);
                    }
                    
                    let target_block = *block_map.get(&target_pc).unwrap();
                    func_builder.ins().jump(target_block, &[]);
                    
                    // Seal current block
                    let current_block = func_builder.current_block().unwrap();
                    if !sealed_blocks.contains(&current_block) {
                        func_builder.seal_block(current_block);
                        sealed_blocks.insert(current_block);
                    }
                    
                    // Switch to target block
                    func_builder.switch_to_block(target_block);
                    pc += 1;
                    compiled_instructions += 1;
                    continue;
                }
                _ => {}
            }

            // Compile this instruction
            Self::compile_instruction_static(
                &mut func_builder,
                instr,
                opcode,
                registers_ptr,
                constants_ptr,
                pointer_type,
                value_size,
            )?;

            pc += 1;
            compiled_instructions += 1;
        }
        
        // Seal all blocks that haven't been sealed
        for &block in block_map.values() {
            if !sealed_blocks.contains(&block) {
                func_builder.seal_block(block);
            }
        }

        // Return the next PC
        let return_val = func_builder.ins().iconst(types::I32, pc as i64);
        func_builder.ins().return_(&[return_val]);

        // Finalize function
        func_builder.finalize();

        // Compile to machine code
        let mut ctx = codegen::Context::for_function(func);
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define function: {}", e))?;

        self.module.clear_context(&mut ctx);
        self.module.finalize_definitions()
            .map_err(|e| format!("Failed to finalize: {}", e))?;

        // Get function pointer
        let code_ptr = self.module.get_finalized_function(func_id);
        let jit_func: JitFunction = unsafe { mem::transmute(code_ptr) };

        // Cache the compiled function
        self.compiled_cache
            .insert((chunk_id, start_pc), (func_id, jit_func));

        Ok(jit_func)
    }

    /// Compile a single instruction to Cranelift IR
    fn compile_instruction_static(
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
                let b_val = builder.ins().load(types::I64, MemFlags::new(), b_addr, 0);

                // Store to R(A)
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                builder.ins().store(MemFlags::new(), b_val, a_addr, 0);
            }

            OpCode::LoadK => {
                let a = Instruction::get_a(instr) as i32;
                let bx = Instruction::get_bx(instr) as i32;

                // Load K(Bx)
                let k_offset = builder.ins().iconst(pointer_type, (bx * value_size) as i64);
                let k_addr = builder.ins().iadd(constants_ptr, k_offset);
                let k_val = builder.ins().load(types::I64, MemFlags::new(), k_addr, 0);

                // Store to R(A)
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                builder.ins().store(MemFlags::new(), k_val, a_addr, 0);
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

                // Store to R(A) - store discriminant and value
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                
                // Store discriminant (3 for Integer variant)
                let discriminant = builder.ins().iconst(types::I64, 3);
                builder.ins().store(MemFlags::new(), discriminant, a_addr, 0);
                
                // Store value
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
                builder.ins().store(MemFlags::new(), discriminant, a_addr, 0);
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
                builder.ins().store(MemFlags::new(), discriminant, a_addr, 0);
                builder.ins().store(MemFlags::new(), result, a_addr, 8);
            }

            OpCode::LoadNil => {
                let a = Instruction::get_a(instr) as i32;
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                
                // Nil discriminant is 0
                let discriminant = builder.ins().iconst(types::I64, 0);
                builder.ins().store(MemFlags::new(), discriminant, a_addr, 0);
            }

            OpCode::LoadBool => {
                let a = Instruction::get_a(instr) as i32;
                let b = Instruction::get_b(instr);
                
                let a_offset = builder.ins().iconst(pointer_type, (a * value_size) as i64);
                let a_addr = builder.ins().iadd(registers_ptr, a_offset);
                
                // Boolean discriminant is 1
                let discriminant = builder.ins().iconst(types::I64, 1);
                builder.ins().store(MemFlags::new(), discriminant, a_addr, 0);
                
                // Store boolean value
                let bool_val = builder.ins().iconst(types::I64, if b != 0 { 1 } else { 0 });
                builder.ins().store(MemFlags::new(), bool_val, a_addr, 8);
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
    pub fn get_compiled(&self, chunk_id: usize, pc: usize) -> Option<JitFunction> {
        self.compiled_cache.get(&(chunk_id, pc)).map(|&(_, f)| f)
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

    #[test]
    fn test_hot_path_tracker() {
        let mut tracker = HotPathTracker::new();
        
        // Record below threshold
        for _ in 0..HOT_THRESHOLD - 1 {
            assert!(!tracker.record(0));
        }
        
        // Should become hot
        assert!(tracker.record(0));
        assert!(tracker.is_hot(0));
    }
}
