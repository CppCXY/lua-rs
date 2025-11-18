// SSA-based optimizations for Lua bytecode

use super::ssa_builder::*;
use super::{OptConfig, OptimizedFunction};
use crate::lua_value::LuaValue;
use std::collections::HashMap;

/// Apply all enabled optimizations
pub fn apply_optimizations(mut ssa: SSAFunction, config: &OptConfig) -> SSAFunction {
    // Run optimizations in order
    if config.constant_folding {
        ssa = constant_folding(ssa);
    }
    
    if config.copy_propagation {
        ssa = copy_propagation(ssa);
    }
    
    if config.algebraic_simplification {
        ssa = algebraic_simplification(ssa);
    }
    
    if config.strength_reduction {
        ssa = strength_reduction(ssa);
    }
    
    if config.dead_code_elimination {
        ssa = dead_code_elimination(ssa);
    }
    
    ssa
}

/// Constant folding: evaluate operations on constants at compile time
fn constant_folding(mut ssa: SSAFunction) -> SSAFunction {
    let mut constants = HashMap::new();
    
    for block in &mut ssa.blocks {
        let mut new_instructions = Vec::new();
        
        for instr in &block.instructions {
            match instr {
                SSAInstruction::ConstInt(dst, val) => {
                    constants.insert(*dst, ConstValue::Int(*val));
                    new_instructions.push(instr.clone());
                }
                
                SSAInstruction::ConstFloat(dst, val) => {
                    constants.insert(*dst, ConstValue::Float(*val));
                    new_instructions.push(instr.clone());
                }
                
                // Fold integer additions
                SSAInstruction::AddII(dst, lhs, rhs) => {
                    if let (Some(ConstValue::Int(l)), Some(ConstValue::Int(r))) = 
                        (constants.get(lhs), constants.get(rhs)) {
                        let result = l + r;
                        constants.insert(*dst, ConstValue::Int(result));
                        new_instructions.push(SSAInstruction::ConstInt(*dst, result));
                    } else {
                        new_instructions.push(instr.clone());
                    }
                }
                
                // Fold integer subtractions
                SSAInstruction::SubII(dst, lhs, rhs) => {
                    if let (Some(ConstValue::Int(l)), Some(ConstValue::Int(r))) = 
                        (constants.get(lhs), constants.get(rhs)) {
                        let result = l - r;
                        constants.insert(*dst, ConstValue::Int(result));
                        new_instructions.push(SSAInstruction::ConstInt(*dst, result));
                    } else {
                        new_instructions.push(instr.clone());
                    }
                }
                
                // Fold integer multiplications
                SSAInstruction::MulII(dst, lhs, rhs) => {
                    if let (Some(ConstValue::Int(l)), Some(ConstValue::Int(r))) = 
                        (constants.get(lhs), constants.get(rhs)) {
                        let result = l * r;
                        constants.insert(*dst, ConstValue::Int(result));
                        new_instructions.push(SSAInstruction::ConstInt(*dst, result));
                    } else {
                        new_instructions.push(instr.clone());
                    }
                }
                
                _ => {
                    new_instructions.push(instr.clone());
                }
            }
        }
        
        block.instructions = new_instructions;
    }
    
    ssa
}

/// Copy propagation: replace uses of copied values with the original
fn copy_propagation(mut ssa: SSAFunction) -> SSAFunction {
    let mut copies = HashMap::new();
    
    for block in &mut ssa.blocks {
        let mut new_instructions = Vec::new();
        
        for instr in &block.instructions {
            match instr {
                SSAInstruction::Copy(dst, src) => {
                    // Track copy
                    copies.insert(*dst, *src);
                    new_instructions.push(instr.clone());
                }
                
                // Replace copied values in arithmetic
                SSAInstruction::AddII(dst, lhs, rhs) => {
                    let lhs = copies.get(lhs).copied().unwrap_or(*lhs);
                    let rhs = copies.get(rhs).copied().unwrap_or(*rhs);
                    new_instructions.push(SSAInstruction::AddII(*dst, lhs, rhs));
                }
                
                SSAInstruction::SubII(dst, lhs, rhs) => {
                    let lhs = copies.get(lhs).copied().unwrap_or(*lhs);
                    let rhs = copies.get(rhs).copied().unwrap_or(*rhs);
                    new_instructions.push(SSAInstruction::SubII(*dst, lhs, rhs));
                }
                
                SSAInstruction::MulII(dst, lhs, rhs) => {
                    let lhs = copies.get(lhs).copied().unwrap_or(*lhs);
                    let rhs = copies.get(rhs).copied().unwrap_or(*rhs);
                    new_instructions.push(SSAInstruction::MulII(*dst, lhs, rhs));
                }
                
                _ => {
                    new_instructions.push(instr.clone());
                }
            }
        }
        
        block.instructions = new_instructions;
    }
    
    ssa
}

/// Algebraic simplification: x + 0 = x, x * 1 = x, x * 0 = 0, etc.
fn algebraic_simplification(mut ssa: SSAFunction) -> SSAFunction {
    let mut constants = HashMap::new();
    
    // First, collect all constants
    for block in &ssa.blocks {
        for instr in &block.instructions {
            match instr {
                SSAInstruction::ConstInt(dst, val) => {
                    constants.insert(*dst, ConstValue::Int(*val));
                }
                SSAInstruction::ConstFloat(dst, val) => {
                    constants.insert(*dst, ConstValue::Float(*val));
                }
                _ => {}
            }
        }
    }
    
    // Apply simplifications
    for block in &mut ssa.blocks {
        let mut new_instructions = Vec::new();
        
        for instr in &block.instructions {
            match instr {
                // x + 0 = x
                SSAInstruction::AddII(dst, lhs, rhs) => {
                    if let Some(ConstValue::Int(0)) = constants.get(rhs) {
                        new_instructions.push(SSAInstruction::Copy(*dst, *lhs));
                    } else if let Some(ConstValue::Int(0)) = constants.get(lhs) {
                        new_instructions.push(SSAInstruction::Copy(*dst, *rhs));
                    } else {
                        new_instructions.push(instr.clone());
                    }
                }
                
                // x - 0 = x
                SSAInstruction::SubII(dst, lhs, rhs) => {
                    if let Some(ConstValue::Int(0)) = constants.get(rhs) {
                        new_instructions.push(SSAInstruction::Copy(*dst, *lhs));
                    } else {
                        new_instructions.push(instr.clone());
                    }
                }
                
                // x * 0 = 0, x * 1 = x
                SSAInstruction::MulII(dst, lhs, rhs) => {
                    if let Some(ConstValue::Int(0)) = constants.get(rhs) {
                        new_instructions.push(SSAInstruction::ConstInt(*dst, 0));
                    } else if let Some(ConstValue::Int(0)) = constants.get(lhs) {
                        new_instructions.push(SSAInstruction::ConstInt(*dst, 0));
                    } else if let Some(ConstValue::Int(1)) = constants.get(rhs) {
                        new_instructions.push(SSAInstruction::Copy(*dst, *lhs));
                    } else if let Some(ConstValue::Int(1)) = constants.get(lhs) {
                        new_instructions.push(SSAInstruction::Copy(*dst, *rhs));
                    } else {
                        new_instructions.push(instr.clone());
                    }
                }
                
                _ => {
                    new_instructions.push(instr.clone());
                }
            }
        }
        
        block.instructions = new_instructions;
    }
    
    ssa
}

/// Strength reduction: x * 2 => x << 1, x / 2 => x >> 1
fn strength_reduction(mut ssa: SSAFunction) -> SSAFunction {
    let mut constants = HashMap::new();
    
    for block in &ssa.blocks {
        for instr in &block.instructions {
            if let SSAInstruction::ConstInt(dst, val) = instr {
                constants.insert(*dst, *val);
            }
        }
    }
    
    for block in &mut ssa.blocks {
        let mut new_instructions = Vec::new();
        
        for instr in &block.instructions {
            match instr {
                // x * power_of_2 => x << log2(power_of_2)
                SSAInstruction::MulII(dst, lhs, rhs) => {
                    if let Some(&val) = constants.get(rhs) {
                        if val > 0 && (val & (val - 1)) == 0 {
                            // is power of two
                            let shift = val.trailing_zeros() as i64;
                            let shift_val = SSAValue(ssa.value_count);
                            new_instructions.push(SSAInstruction::ConstInt(shift_val, shift));
                            // Note: Would need Shl instruction, skipping for now
                            new_instructions.push(instr.clone());
                        } else {
                            new_instructions.push(instr.clone());
                        }
                    } else {
                        new_instructions.push(instr.clone());
                    }
                }
                
                _ => {
                    new_instructions.push(instr.clone());
                }
            }
        }
        
        block.instructions = new_instructions;
    }
    
    ssa
}

/// Dead code elimination: remove unused values
fn dead_code_elimination(mut ssa: SSAFunction) -> SSAFunction {
    let mut used = std::collections::HashSet::new();
    
    // Mark all used values
    for block in &ssa.blocks {
        for instr in &block.instructions {
            match instr {
                SSAInstruction::AddII(_, lhs, rhs) |
                SSAInstruction::SubII(_, lhs, rhs) |
                SSAInstruction::MulII(_, lhs, rhs) => {
                    used.insert(*lhs);
                    used.insert(*rhs);
                }
                SSAInstruction::Copy(_, src) => {
                    used.insert(*src);
                }
                SSAInstruction::Return(vals) => {
                    for val in vals {
                        used.insert(*val);
                    }
                }
                _ => {}
            }
        }
    }
    
    // Remove unused instructions
    for block in &mut ssa.blocks {
        block.instructions.retain(|instr| {
            match instr {
                SSAInstruction::ConstInt(dst, _) |
                SSAInstruction::ConstFloat(dst, _) |
                SSAInstruction::ConstNil(dst) |
                SSAInstruction::AddII(dst, _, _) |
                SSAInstruction::SubII(dst, _, _) |
                SSAInstruction::MulII(dst, _, _) |
                SSAInstruction::Copy(dst, _) => used.contains(dst),
                _ => true, // Keep other instructions
            }
        });
    }
    
    ssa
}

/// Convert SSA back to bytecode
pub fn ssa_to_bytecode(ssa: SSAFunction, constants: &[LuaValue]) -> OptimizedFunction {
    use crate::opcode::{Instruction, OpCode};
    use std::collections::HashMap;
    
    // Map SSA values to registers
    let mut value_to_register = HashMap::new();
    let mut next_register = 0u32;
    
    let mut instructions = Vec::new();
    let mut new_constants = constants.to_vec();
    
    // Helper to get or allocate register for SSA value
    let mut get_register = |val: SSAValue, map: &mut HashMap<SSAValue, u32>| -> u32 {
        *map.entry(val).or_insert_with(|| {
            let reg = next_register;
            next_register += 1;
            reg
        })
    };
    
    // Process each block
    for block in &ssa.blocks {
        for instr in &block.instructions {
            match instr {
                SSAInstruction::ConstInt(dst, val) => {
                    // Add constant to pool if not exists
                    let const_idx = new_constants.len();
                    new_constants.push(LuaValue::integer(*val));
                    let reg = get_register(*dst, &mut value_to_register);
                    instructions.push(Instruction::encode_abx(OpCode::LoadK, reg, const_idx as u32));
                }
                
                SSAInstruction::ConstFloat(dst, val) => {
                    let const_idx = new_constants.len();
                    new_constants.push(LuaValue::float(*val));
                    let reg = get_register(*dst, &mut value_to_register);
                    instructions.push(Instruction::encode_abx(OpCode::LoadK, reg, const_idx as u32));
                }
                
                SSAInstruction::ConstNil(dst) => {
                    let reg = get_register(*dst, &mut value_to_register);
                    instructions.push(Instruction::encode_abc(OpCode::LoadNil, reg, 0, 0));
                }
                
                SSAInstruction::AddII(dst, lhs, rhs) |
                SSAInstruction::Add(dst, lhs, rhs) => {
                    let dst_reg = get_register(*dst, &mut value_to_register);
                    let lhs_reg = get_register(*lhs, &mut value_to_register);
                    let rhs_reg = get_register(*rhs, &mut value_to_register);
                    instructions.push(Instruction::encode_abc(OpCode::Add, dst_reg, lhs_reg, rhs_reg));
                }
                
                SSAInstruction::SubII(dst, lhs, rhs) |
                SSAInstruction::Sub(dst, lhs, rhs) => {
                    let dst_reg = get_register(*dst, &mut value_to_register);
                    let lhs_reg = get_register(*lhs, &mut value_to_register);
                    let rhs_reg = get_register(*rhs, &mut value_to_register);
                    instructions.push(Instruction::encode_abc(OpCode::Sub, dst_reg, lhs_reg, rhs_reg));
                }
                
                SSAInstruction::MulII(dst, lhs, rhs) |
                SSAInstruction::Mul(dst, lhs, rhs) => {
                    let dst_reg = get_register(*dst, &mut value_to_register);
                    let lhs_reg = get_register(*lhs, &mut value_to_register);
                    let rhs_reg = get_register(*rhs, &mut value_to_register);
                    instructions.push(Instruction::encode_abc(OpCode::Mul, dst_reg, lhs_reg, rhs_reg));
                }
                
                SSAInstruction::DivII(dst, lhs, rhs) |
                SSAInstruction::Div(dst, lhs, rhs) => {
                    let dst_reg = get_register(*dst, &mut value_to_register);
                    let lhs_reg = get_register(*lhs, &mut value_to_register);
                    let rhs_reg = get_register(*rhs, &mut value_to_register);
                    instructions.push(Instruction::encode_abc(OpCode::Div, dst_reg, lhs_reg, rhs_reg));
                }
                
                SSAInstruction::Copy(dst, src) => {
                    let dst_reg = get_register(*dst, &mut value_to_register);
                    let src_reg = get_register(*src, &mut value_to_register);
                    instructions.push(Instruction::encode_abc(OpCode::Move, dst_reg, src_reg, 0));
                }
                
                // Skip other instructions for now
                _ => {}
            }
        }
    }
    
    OptimizedFunction {
        instructions,
        constants: new_constants,
        max_stack: next_register as usize,
    }
}

#[derive(Debug, Clone)]
enum ConstValue {
    Int(i64),
    Float(f64),
}
