// IO library (stub implementation)
// Implements: close, flush, input, lines, open, output, popen, read, 
// tmpfile, type, write

use crate::lib_registry::LibraryModule;
use crate::value::{LuaValue, MultiValue};
use crate::vm::VM;

pub fn create_io_lib() -> LibraryModule {
    crate::lib_module!("io", {
        "write" => io_write,
        "read" => io_read,
        "flush" => io_flush,
    })
}

fn io_write(vm: &mut VM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);
    
    for arg in args {
        if let Some(s) = arg.as_string() {
            print!("{}", s.as_str());
        } else if let Some(n) = arg.as_number() {
            print!("{}", n);
        }
    }
    
    Ok(MultiValue::empty())
}

fn io_read(vm: &mut VM) -> Result<MultiValue, String> {
    // Stub: return nil
    Ok(MultiValue::single(LuaValue::Nil))
}

fn io_flush(_vm: &mut VM) -> Result<MultiValue, String> {
    use std::io::{self, Write};
    io::stdout().flush().ok();
    Ok(MultiValue::empty())
}

