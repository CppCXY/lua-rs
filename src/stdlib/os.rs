// OS library (stub implementation)
// Implements: clock, date, difftime, execute, exit, getenv, remove, rename, 
// setlocale, time, tmpname

use crate::lib_registry::LibraryModule;
use crate::value::{LuaValue, MultiValue};
use crate::vm::VM;

pub fn create_os_lib() -> LibraryModule {
    crate::lib_module!("os", {
        "clock" => os_clock,
        "time" => os_time,
        "date" => os_date,
        "exit" => os_exit,
    })
}

fn os_clock(_vm: &mut VM) -> Result<MultiValue, String> {
    use std::time::SystemTime;
    
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    
    let secs = duration.as_secs_f64();
    Ok(MultiValue::single(LuaValue::Float(secs)))
}

fn os_time(_vm: &mut VM) -> Result<MultiValue, String> {
    use std::time::SystemTime;
    
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    Ok(MultiValue::single(LuaValue::Integer(timestamp as i64)))
}

fn os_date(vm: &mut VM) -> Result<MultiValue, String> {
    // Stub: return current timestamp as string
    use std::time::SystemTime;
    
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    let date_str = format!("timestamp: {}", timestamp);
    let result = vm.create_string(date_str);
    Ok(MultiValue::single(LuaValue::String(result)))
}

fn os_exit(_vm: &mut VM) -> Result<MultiValue, String> {
    std::process::exit(0);
}

