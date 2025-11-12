use lua_rt::{Compiler, VM};
use std::rc::Rc;

fn main() {
    println!("Testing Lua compiler and VM\n");

    // Test 1: Simple arithmetic
    test_code("Test 1: Arithmetic", "local a = 10\nlocal b = 20\nreturn a + b");
    
    // Test 2: Variables
    test_code("Test 2: Variables", "local x = 42\nreturn x");
    
    // Test 3: Boolean
    test_code("Test 3: Boolean", "return true");
    
    // Test 4: Nil
    test_code("Test 4: Nil", "return nil");
    
    // Test 5: Multiple operations
    test_code("Test 5: Multiple ops", "local a = 5\nlocal b = 3\nlocal c = a * b\nreturn c");
    
    // Test 6: Subtraction
    test_code("Test 6: Subtraction", "local x = 100\nlocal y = 42\nreturn x - y");
    
    println!("\nAll tests completed!");
}

fn test_code(name: &str, code: &str) {
    println!("{}", name);
    println!("Code: {}", code);
    
    match Compiler::compile(code) {
        Ok(chunk) => {
            println!("✓ Compilation successful");
            println!("  Constants: {}", chunk.constants.len());
            println!("  Instructions: {}", chunk.code.len());
            println!("  Max stack: {}", chunk.max_stack_size);
            
            // Try to execute
            let mut vm = VM::new();
            match vm.execute(Rc::new(chunk)) {
                Ok(result) => {
                    println!("✓ Execution successful");
                    println!("  Result: {:?}", result);
                }
                Err(e) => {
                    println!("✗ Runtime error: {}", e);
                }
            }
        }
        Err(e) => {
            println!("✗ Compile error: {}", e);
        }
    }
    println!();
}
