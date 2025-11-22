// Example: Using async functions in Lua coroutines
// This demonstrates how Lua coroutines can call Rust async functions

use lua_rs::LuaVM;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

fn main() {
    println!("=== Lua Async Example ===\n");

    // Create VM and load standard libraries
    let mut vm = LuaVM::new();
    vm.open_libs();

    // Lua code that uses async functions
    let lua_code = r#"
        print("Creating coroutine...")
        local co = coroutine.create(function()
            print("  [Coroutine] Starting")
            print("  [Coroutine] Calling async.sleep(100)...")
            async.sleep(100)
            print("  [Coroutine] Woke up after 100ms")
            
            print("  [Coroutine] Calling async.sleep(200)...")
            async.sleep(200)
            print("  [Coroutine] Woke up after 200ms")
            
            print("  [Coroutine] Done!")
            return "completed"
        end)
        
        print("Starting coroutine...")
        local success, value = coroutine.resume(co)
        print("First resume returned:", success, value)
        
        -- Return the coroutine so Rust can continue resuming it
        return co
    "#;

    // Compile and execute the Lua code
    let chunk = match vm.compile(lua_code) {
        Ok(chunk) => Rc::new(chunk),
        Err(e) => {
            eprintln!("Compile error: {}", e);
            return;
        }
    };

    match vm.execute(chunk) {
        Ok(_result) => {
            println!("\nLua script executed successfully");
            
            // Now poll async tasks and resume coroutines
            println!("\nPolling async tasks...");
            for i in 1..=10 {
                // Poll every 50ms
                thread::sleep(Duration::from_millis(50));
                
                match vm.poll_async() {
                    Ok(_) => {
                        let active_count = vm.active_async_tasks();
                        if active_count > 0 {
                            println!("  [Poll #{}] Active tasks: {}", i, active_count);
                        } else {
                            println!("  [Poll #{}] All tasks completed!", i);
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("  [Poll #{}] Error: {}", i, e);
                        break;
                    }
                }
            }
            
            println!("\nAsync example completed!");
        }
        Err(e) => {
            eprintln!("Execution error: {}", e);
        }
    }
}
