// Simple constant folding optimizer
// Works directly on bytecode without full SSA conversion

use crate::lua_value::LuaValue;
use crate::opcode::{Instruction, OpCode};
use std::collections::HashMap;

/// Simple optimization pass that folds constants
pub fn optimize_constants(
    instructions: &[u32],
    constants: &[LuaValue],
) -> (Vec<u32>, Vec<LuaValue>) {
    let mut new_instructions = Vec::new();
    let mut new_constants = constants.to_vec();
    let mut register_values: HashMap<u32, ConstValue> = HashMap::new();
    
    for &instr in instructions {
        let opcode = Instruction::get_opcode(instr);
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr);
        let c = Instruction::get_c(instr);
        let bx = Instruction::get_bx(instr);
        
        match opcode {
            OpCode::LoadK => {
                // Track constant loads
                if let Some(const_val) = constants.get(bx as usize) {
                    if let Some(i) = const_val.as_integer() {
                        register_values.insert(a, ConstValue::Int(i));
                    } else if let Some(f) = const_val.as_float() {
                        register_values.insert(a, ConstValue::Float(f));
                    }
                }
                new_instructions.push(instr);
            }
            
            OpCode::Add => {
                // Try to fold constant addition
                if let (Some(ConstValue::Int(l)), Some(ConstValue::Int(r))) = 
                    (register_values.get(&b), register_values.get(&c)) {
                    // Both operands are known integer constants
                    let result = l + r;
                    let const_idx = new_constants.len();
                    new_constants.push(LuaValue::integer(result));
                    register_values.insert(a, ConstValue::Int(result));
                    
                    // Replace Add with LoadK
                    new_instructions.push(Instruction::encode_abx(OpCode::LoadK, a, const_idx as u32));
                } else {
                    // Can't optimize, keep original
                    register_values.remove(&a);
                    new_instructions.push(instr);
                }
            }
            
            OpCode::Sub => {
                if let (Some(ConstValue::Int(l)), Some(ConstValue::Int(r))) = 
                    (register_values.get(&b), register_values.get(&c)) {
                    let result = l - r;
                    let const_idx = new_constants.len();
                    new_constants.push(LuaValue::integer(result));
                    register_values.insert(a, ConstValue::Int(result));
                    new_instructions.push(Instruction::encode_abx(OpCode::LoadK, a, const_idx as u32));
                } else {
                    register_values.remove(&a);
                    new_instructions.push(instr);
                }
            }
            
            OpCode::Mul => {
                if let (Some(ConstValue::Int(l)), Some(ConstValue::Int(r))) = 
                    (register_values.get(&b), register_values.get(&c)) {
                    let result = l * r;
                    let const_idx = new_constants.len();
                    new_constants.push(LuaValue::integer(result));
                    register_values.insert(a, ConstValue::Int(result));
                    new_instructions.push(Instruction::encode_abx(OpCode::LoadK, a, const_idx as u32));
                } else {
                    register_values.remove(&a);
                    new_instructions.push(instr);
                }
            }
            
            OpCode::Move => {
                // Propagate constant values through moves
                if let Some(val) = register_values.get(&b).cloned() {
                    register_values.insert(a, val);
                } else {
                    register_values.remove(&a);
                }
                new_instructions.push(instr);
            }
            
            // Control flow instructions - clear all register tracking
            OpCode::Jmp | OpCode::ForLoop | OpCode::ForPrep | 
            OpCode::Lt | OpCode::Le | OpCode::Eq | OpCode::Ne | OpCode::Gt | OpCode::Ge |
            OpCode::Test | OpCode::TestSet => {
                // Clear all tracked values on control flow changes
                // because registers may have different values on different paths
                register_values.clear();
                new_instructions.push(instr);
            }
            
            _ => {
                // For all other instructions, clear affected registers and keep instruction
                register_values.remove(&a);
                new_instructions.push(instr);
            }
        }
    }
    
    (new_instructions, new_constants)
}

#[derive(Debug, Clone)]
enum ConstValue {
    Int(i64),
    Float(f64),
}
