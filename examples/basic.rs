// Example: Using the Lua VM

use lua_rt::{VM, Compiler, LuaValue};
use std::rc::Rc;

fn main() {
    println!("=== Lua VM Example ===\n");

    // Create a new VM instance
    let mut vm = VM::new();

    // Compile Lua code
    println!("Compiling Lua code...");
    let source = "return 42";  // Simple test program
    
    match Compiler::compile(source) {
        Ok(chunk) => {
            println!("✓ Compilation successful!");
            println!("  - {} instructions", chunk.code.len());
            println!("  - {} constants", chunk.constants.len());
            println!("  - Max stack size: {}\n", chunk.max_stack_size);

            // Execute the bytecode
            println!("Executing bytecode...");
            match vm.execute(Rc::new(chunk)) {
                Ok(result) => {
                    println!("✓ Execution successful!");
                    // println!("  Result: {:?}\n", result);

                    // Verify the result
                    if let Some(num) = result.as_number() {
                        println!("  The result is a number: {}\n", num);
                    }
                }
                Err(e) => {
                    eprintln!("✗ Runtime error: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("✗ Compilation error: {}", e);
        }
    }

    // Demonstrate value types
    println!("=== Lua Value Types ===");
    
    println!("Nil: {}", if LuaValue::nil().is_nil() { "nil" } else { "?" });
    println!("Boolean: {}", if LuaValue::boolean(true).as_boolean().unwrap() { "true" } else { "false" });
    
    let num_val = LuaValue::number(3.14159);
    if let Some(n) = num_val.as_number() {
        println!("Number: {}", n);
    }

    // Check memory size
    println!("\n=== Memory Layout ===");
    println!("Size of LuaValue: {} bytes", std::mem::size_of::<LuaValue>());
    println!("(Thanks to NaN-boxing, all values fit in 8 bytes!)");

    println!("\n=== Done ===");
}
