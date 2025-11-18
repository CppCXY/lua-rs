// Check sizes of critical data structures
use lua_rs::*;

fn main() {
    println!("=== Critical Data Structure Sizes ===\n");

    println!("LuaValue:        {} bytes", std::mem::size_of::<LuaValue>());
    println!(
        "LuaCallFrame:    {} bytes ← OPTIMIZED!",
        std::mem::size_of::<lua_vm::LuaCallFrame>()
    );
    println!("LuaTable:        {} bytes", std::mem::size_of::<LuaTable>());
    println!(
        "LuaFunction:     {} bytes",
        std::mem::size_of::<LuaFunction>()
    );

    println!("\n=== Memory Savings ===");
    println!("Original LuaCallFrame: 80 bytes");
    println!(
        "Optimized LuaCallFrame: {} bytes",
        std::mem::size_of::<lua_vm::LuaCallFrame>()
    );
    let savings = 80 - std::mem::size_of::<lua_vm::LuaCallFrame>();
    let percent = (savings as f64 / 80.0) * 100.0;
    println!("Savings: {} bytes ({:.1}% reduction)", savings, percent);

    println!("\n=== Call Stack Impact ===");
    let frame_size = std::mem::size_of::<lua_vm::LuaCallFrame>();
    println!("Typical call depth: 100 frames");
    println!("Old memory: 100 × 80 bytes = 8000 bytes");
    println!(
        "New memory: 100 × {} bytes = {} bytes",
        frame_size,
        100 * frame_size
    );
    println!("Stack savings: {} bytes", 8000 - 100 * frame_size);

    println!("\n=== Cache Line Efficiency ===");
    println!("Cache line size: 64 bytes");
    println!("Frames per cache line (old): {:.2}", 64.0 / 80.0);
    println!(
        "Frames per cache line (new): {:.2}",
        64.0 / frame_size as f64
    );
    println!(
        "Cache efficiency boost: {:.1}x",
        (64.0 / frame_size as f64) / (64.0 / 80.0)
    );
}
