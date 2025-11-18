// SSA (Static Single Assignment) builder for Lua bytecode
// Converts bytecode to SSA form for optimization

use crate::opcode::{OpCode, Instruction};
use crate::lua_value::LuaValue;
use std::collections::{HashMap, HashSet};

/// SSA value identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SSAValue(pub usize);

/// SSA instruction in IR form
#[derive(Debug, Clone)]
pub enum SSAInstruction {
    // Constants
    ConstInt(SSAValue, i64),
    ConstFloat(SSAValue, f64),
    ConstNil(SSAValue),
    ConstBool(SSAValue, bool),
    ConstString(SSAValue, usize), // index into constant pool
    
    // Arithmetic (integer specialized)
    AddII(SSAValue, SSAValue, SSAValue),  // dst, lhs, rhs
    SubII(SSAValue, SSAValue, SSAValue),
    MulII(SSAValue, SSAValue, SSAValue),
    DivII(SSAValue, SSAValue, SSAValue),
    ModII(SSAValue, SSAValue, SSAValue),
    
    // Arithmetic (float specialized)
    AddFF(SSAValue, SSAValue, SSAValue),
    SubFF(SSAValue, SSAValue, SSAValue),
    MulFF(SSAValue, SSAValue, SSAValue),
    DivFF(SSAValue, SSAValue, SSAValue),
    ModFF(SSAValue, SSAValue, SSAValue),
    
    // Generic (handles type dispatch)
    Add(SSAValue, SSAValue, SSAValue),
    Sub(SSAValue, SSAValue, SSAValue),
    Mul(SSAValue, SSAValue, SSAValue),
    Div(SSAValue, SSAValue, SSAValue),
    Mod(SSAValue, SSAValue, SSAValue),
    
    // Comparisons
    EqII(SSAValue, SSAValue, SSAValue),
    LtII(SSAValue, SSAValue, SSAValue),
    LeII(SSAValue, SSAValue, SSAValue),
    
    // Copy
    Copy(SSAValue, SSAValue),  // dst = src
    
    // Phi nodes (for control flow merge)
    Phi(SSAValue, Vec<SSAValue>),
    
    // Control flow
    Branch(SSAValue, usize, usize), // condition, true_block, false_block
    Jump(usize),                     // target_block
    Return(Vec<SSAValue>),
    
    // Table operations
    NewTable(SSAValue),
    GetTable(SSAValue, SSAValue, SSAValue), // dst, table, key
    SetTable(SSAValue, SSAValue, SSAValue), // table, key, value
    
    // Function calls
    Call(SSAValue, Vec<SSAValue>), // dst (or first result), args
    
    // Type guards (for specialization)
    GuardInteger(SSAValue),
    GuardFloat(SSAValue),
    GuardTable(SSAValue),
}

/// Basic block in SSA form
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: usize,
    pub instructions: Vec<SSAInstruction>,
    pub predecessors: Vec<usize>,
    pub successors: Vec<usize>,
}

/// SSA function representation
#[derive(Debug, Clone)]
pub struct SSAFunction {
    pub blocks: Vec<BasicBlock>,
    pub entry_block: usize,
    pub value_count: usize,
    pub register_count: usize,
}

/// Builder for converting bytecode to SSA
pub struct SSABuilder {
    next_value: usize,
    blocks: Vec<BasicBlock>,
    current_block: usize,
    register_versions: HashMap<u8, SSAValue>, // maps Lua register to current SSA value
}

impl SSABuilder {
    pub fn new() -> Self {
        Self {
            next_value: 0,
            blocks: vec![BasicBlock {
                id: 0,
                instructions: Vec::new(),
                predecessors: Vec::new(),
                successors: Vec::new(),
            }],
            current_block: 0,
            register_versions: HashMap::new(),
        }
    }
    
    fn new_value(&mut self) -> SSAValue {
        let val = SSAValue(self.next_value);
        self.next_value += 1;
        val
    }
    
    fn get_register(&mut self, reg: u8) -> SSAValue {
        if let Some(&val) = self.register_versions.get(&reg) {
            val
        } else {
            let val = self.new_value();
            self.register_versions.insert(reg, val);
            val
        }
    }
    
    fn set_register(&mut self, reg: u8, val: SSAValue) {
        self.register_versions.insert(reg, val);
    }
    
    fn emit(&mut self, instr: SSAInstruction) {
        self.blocks[self.current_block].instructions.push(instr);
    }
    
    pub fn from_bytecode(bytecode: &[u32], constants: &[LuaValue]) -> SSAFunction {
        let mut builder = SSABuilder::new();
        
        // First pass: identify basic blocks (split at jumps and branch targets)
        let mut block_starts = HashSet::new();
        block_starts.insert(0);
        
        for (pc, &instr) in bytecode.iter().enumerate() {
            let opcode = Instruction::get_opcode(instr);
            
            match opcode {
                OpCode::Jmp => {
                    let sbx = Instruction::get_sbx(instr);
                    let target = (pc as i32 + 1 + sbx) as usize;
                    block_starts.insert(target);
                    block_starts.insert(pc + 1);
                }
                OpCode::ForPrep | OpCode::ForLoop => {
                    let sbx = Instruction::get_sbx(instr);
                    let target = (pc as i32 + 1 + sbx) as usize;
                    block_starts.insert(target);
                    block_starts.insert(pc + 1);
                }
                OpCode::Test | OpCode::TestSet | OpCode::Eq | OpCode::Lt | OpCode::Le => {
                    block_starts.insert(pc + 1);
                    block_starts.insert(pc + 2);
                }
                _ => {}
            }
        }
        
        // Second pass: convert to SSA
        // For simple linear code, just process instructions sequentially
        for (pc, &instr) in bytecode.iter().enumerate() {
            builder.convert_instruction(instr, pc, constants);
        }
        
        SSAFunction {
            blocks: builder.blocks,
            entry_block: 0,
            value_count: builder.next_value,
            register_count: builder.register_versions.len(),
        }
    }
    
    fn convert_instruction(&mut self, instr: u32, pc: usize, constants: &[LuaValue]) {
        let opcode = Instruction::get_opcode(instr);
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr);
        let c = Instruction::get_c(instr);
        let bx = Instruction::get_bx(instr);
        
        match opcode {
            OpCode::Move => {
                let src = self.get_register(b as u8);
                let dst = self.new_value();
                self.emit(SSAInstruction::Copy(dst, src));
                self.set_register(a as u8, dst);
            }
            
            OpCode::LoadK => {
                let dst = self.new_value();
                
                // Try to specialize based on constant type
                if let Some(constant) = constants.get(bx as usize) {
                    if let Some(i) = constant.as_integer() {
                        self.emit(SSAInstruction::ConstInt(dst, i));
                    } else if let Some(f) = constant.as_float() {
                        self.emit(SSAInstruction::ConstFloat(dst, f));
                    } else if constant.is_string() {
                        self.emit(SSAInstruction::ConstString(dst, bx as usize));
                    } else {
                        // Generic constant load
                        self.emit(SSAInstruction::ConstString(dst, bx as usize));
                    }
                } else {
                    self.emit(SSAInstruction::ConstNil(dst));
                }
                
                self.set_register(a as u8, dst);
            }
            
            OpCode::LoadNil => {
                let dst = self.new_value();
                self.emit(SSAInstruction::ConstNil(dst));
                self.set_register(a as u8, dst);
            }
            
            OpCode::Add => {
                let lhs = self.get_register(b as u8);
                let rhs = self.get_register(c as u8);
                let dst = self.new_value();
                
                // Generic add (optimizer will specialize if possible)
                self.emit(SSAInstruction::Add(dst, lhs, rhs));
                self.set_register(a as u8, dst);
            }
            
            OpCode::Sub => {
                let lhs = self.get_register(b as u8);
                let rhs = self.get_register(c as u8);
                let dst = self.new_value();
                self.emit(SSAInstruction::Sub(dst, lhs, rhs));
                self.set_register(a as u8, dst);
            }
            
            OpCode::Mul => {
                let lhs = self.get_register(b as u8);
                let rhs = self.get_register(c as u8);
                let dst = self.new_value();
                self.emit(SSAInstruction::Mul(dst, lhs, rhs));
                self.set_register(a as u8, dst);
            }
            
            OpCode::Div => {
                let lhs = self.get_register(b as u8);
                let rhs = self.get_register(c as u8);
                let dst = self.new_value();
                self.emit(SSAInstruction::Div(dst, lhs, rhs));
                self.set_register(a as u8, dst);
            }
            
            OpCode::ForPrep => {
                // For loop initialization
                // R(A) -= R(A+2)
                let init = self.get_register(a as u8);
                let step = self.get_register((a + 2) as u8);
                let adjusted = self.new_value();
                self.emit(SSAInstruction::SubII(adjusted, init, step));
                self.set_register(a as u8, adjusted);
            }
            
            OpCode::ForLoop => {
                // For loop iteration
                // R(A) += R(A+2)
                let idx = self.get_register(a as u8);
                let step = self.get_register((a + 2) as u8);
                let limit = self.get_register((a + 1) as u8);
                
                let new_idx = self.new_value();
                self.emit(SSAInstruction::AddII(new_idx, idx, step));
                self.set_register(a as u8, new_idx);
                
                // Compare and branch
                let cond = self.new_value();
                self.emit(SSAInstruction::LeII(cond, new_idx, limit));
                
                let sbx = Instruction::get_sbx(instr);
                let target = (pc as i32 + 1 + sbx) as usize;
                self.emit(SSAInstruction::Branch(cond, target, pc + 1));
            }
            
            _ => {
                // For now, skip unsupported opcodes
                // A complete implementation would handle all opcodes
            }
        }
    }
}
