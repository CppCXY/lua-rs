// Test VM.run() basic instruction execution
// 
// This test validates that the new dispatcher architecture works correctly
// by executing basic load, move, and return instructions.

use lua_rs::{LuaVM, LuaValue, Chunk, Instruction, OpCode};
use std::rc::Rc;

fn main() {
    println!("=== Testing VM.run() with basic instructions ===\n");

    // Test 1: LOADI and RETURN
    test_loadi_return();
    
    // Test 2: LOADTRUE, LOADFALSE, LOADNIL
    test_load_constants();
    
    // Test 3: MOVE instruction
    test_move();
    
    // Test 4: LOADK (load constant from table)
    test_loadk();
}

fn test_loadi_return() {
    println!("Test 1: LOADI + RETURN");
    
    let mut vm = LuaVM::new();
    
    // Create a simple chunk: R[0] = 42; return R[0]
    let mut chunk = Chunk::new();
    chunk.max_stack_size = 10; // Set max stack size
    
    // LOADI 0 42: R[0] = 42 (use encode_asbx for sBx format)
    let instr1 = Instruction::encode_asbx(OpCode::LoadI, 0, 42);
    chunk.code.push(instr1);
    
    // RETURN 0 2 0: return R[0]
    let instr2 = Instruction::encode_abc(OpCode::Return, 0, 2, 0);
    chunk.code.push(instr2);
    
    // Create function
    let func = vm.create_function(Rc::new(chunk), vec![]);
    
    // Execute
    match vm.call_function(func, vec![]) {
        Ok(result) => {
            println!("  Result: {:?}", result);
            if let Some(i) = result.as_integer() {
                println!("  ✓ Got integer: {}", i);
                assert_eq!(i, 42, "Expected 42");
            } else {
                println!("  ✗ Expected integer, got: {:?}", result);
            }
        }
        Err(e) => {
            println!("  ✗ Error: {:?}", e);
        }
    }
    println!();
}

fn test_load_constants() {
    println!("Test 2: LOADTRUE, LOADFALSE, LOADNIL");
    
    let mut vm = LuaVM::new();
    let mut chunk = Chunk::new();
    chunk.max_stack_size = 10;
    
    // LOADTRUE 0: R[0] = true
    let instr1 = Instruction::encode_abc(OpCode::LoadTrue, 0, 0, 0);
    chunk.code.push(instr1);
    
    // LOADFALSE 1: R[1] = false
    let instr2 = Instruction::encode_abc(OpCode::LoadFalse, 1, 0, 0);
    chunk.code.push(instr2);
    
    // LOADNIL 2 0: R[2] = nil
    let instr3 = Instruction::encode_abc(OpCode::LoadNil, 2, 0, 0);
    chunk.code.push(instr3);
    
    // RETURN 0 1 0: return (no values)
    let instr4 = Instruction::encode_abc(OpCode::Return, 0, 1, 0);
    chunk.code.push(instr4);
    
    let func = vm.create_function(Rc::new(chunk), vec![]);
    
    match vm.call_function(func, vec![]) {
        Ok(_result) => {
            println!("  ✓ Executed successfully");
            // We can't easily check register values, but no error is good
        }
        Err(e) => {
            println!("  ✗ Error: {:?}", e);
        }
    }
    println!();
}

fn test_move() {
    println!("Test 3: MOVE instruction");
    
    let mut vm = LuaVM::new();
    let mut chunk = Chunk::new();
    chunk.max_stack_size = 10;
    
    // LOADI 0 123: R[0] = 123 (use encode_asbx)
    let instr1 = Instruction::encode_asbx(OpCode::LoadI, 0, 123);
    chunk.code.push(instr1);
    
    // MOVE 1 0: R[1] = R[0]
    let instr2 = Instruction::encode_abc(OpCode::Move, 1, 0, 0);
    chunk.code.push(instr2);
    
    // RETURN 1 2 0: return R[1]
    let instr3 = Instruction::encode_abc(OpCode::Return, 1, 2, 0);
    chunk.code.push(instr3);
    
    let func = vm.create_function(Rc::new(chunk), vec![]);
    
    match vm.call_function(func, vec![]) {
        Ok(result) => {
            println!("  Result: {:?}", result);
            if let Some(i) = result.as_integer() {
                println!("  ✓ Got integer: {}", i);
                assert_eq!(i, 123, "Expected 123");
            } else {
                println!("  ✗ Expected integer, got: {:?}", result);
            }
        }
        Err(e) => {
            println!("  ✗ Error: {:?}", e);
        }
    }
    println!();
}

fn test_loadk() {
    println!("Test 4: LOADK (load constant)");
    
    let mut vm = LuaVM::new();
    let mut chunk = Chunk::new();
    chunk.max_stack_size = 10;
    
    // Add constant to constant table
    chunk.constants.push(LuaValue::number(3.14));
    
    // LOADK 0 0: R[0] = K[0]
    let instr1 = Instruction::encode_abx(OpCode::LoadK, 0, 0);
    chunk.code.push(instr1);
    
    // RETURN 0 2 0: return R[0]
    let instr2 = Instruction::encode_abc(OpCode::Return, 0, 2, 0);
    chunk.code.push(instr2);
    
    let func = vm.create_function(Rc::new(chunk), vec![]);
    
    match vm.call_function(func, vec![]) {
        Ok(result) => {
            println!("  Result: {:?}", result);
            if let Some(n) = result.as_number() {
                println!("  ✓ Got number: {}", n);
                assert!((n - 3.14).abs() < 0.001, "Expected 3.14");
            } else {
                println!("  ✗ Expected number, got: {:?}", result);
            }
        }
        Err(e) => {
            println!("  ✗ Error: {:?}", e);
        }
    }
    println!();
}

