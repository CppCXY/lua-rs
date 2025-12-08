use luars::GcUpvalue;
use luars::LuaValue;
use std::mem::size_of;

fn main() {
    println!("=== Size Check ===");
    println!("LuaValue: {} bytes", size_of::<LuaValue>());
    println!("GcUpvalue: {} bytes", size_of::<GcUpvalue>());
}
