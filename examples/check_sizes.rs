use lua_rs::{LuaFunction, LuaVM, lua_vm::LuaCallFrame};
use std::mem::size_of;

fn main() {
    println!("=== Structure Sizes ===");
    println!("LuaCallFrame: {} bytes", size_of::<LuaCallFrame>());
    println!(
        "  - Rc<LuaFunction>: {} bytes",
        size_of::<std::rc::Rc<LuaFunction>>()
    );
    println!("  - Option<String>: {} bytes", size_of::<Option<String>>());
    println!("  - String: {} bytes", size_of::<String>());
    println!("  - usize: {} bytes", size_of::<usize>());
    println!("  - bool: {} bytes", size_of::<bool>());
    println!();
    println!("Breakdown of LuaCallFrame:");
    println!("  frame_id: {} bytes", size_of::<usize>());
    println!(
        "  function: {} bytes",
        size_of::<std::rc::Rc<LuaFunction>>()
    );
    println!("  pc: {} bytes", size_of::<usize>());
    println!("  base_ptr: {} bytes", size_of::<usize>());
    println!("  top: {} bytes", size_of::<usize>());
    println!("  result_reg: {} bytes", size_of::<usize>());
    println!("  num_results: {} bytes", size_of::<usize>());
    println!("  func_name: {} bytes", size_of::<Option<&'static str>>());
    println!("  source: {} bytes", size_of::<Option<&'static str>>());
    println!("  is_protected: {} bytes", size_of::<bool>());
    println!("  vararg_start: {} bytes", size_of::<usize>());
    println!("  vararg_count: {} bytes", size_of::<usize>());

    let total = size_of::<usize>() * 9
        + size_of::<std::rc::Rc<LuaFunction>>()
        + size_of::<Option<&'static str>>() * 2
        + size_of::<bool>();
    println!();
    println!("Expected size (without padding): {} bytes", total);
    println!("Actual size: {} bytes", size_of::<LuaCallFrame>());
    if size_of::<LuaCallFrame>() >= total {
        println!("Padding: {} bytes", size_of::<LuaCallFrame>() - total);
    }
    println!();
    println!("=== Optimization Results ===");
    println!("Before: 128 bytes");
    println!("After: {} bytes", size_of::<LuaCallFrame>());
    println!(
        "Saved: {} bytes ({:.1}%)",
        128 - size_of::<LuaCallFrame>(),
        (128 - size_of::<LuaCallFrame>()) as f64 / 128.0 * 100.0
    );
}
