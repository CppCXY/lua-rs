// Table library functions

use crate::value::LuaValue;
use crate::vm::VM;

/// Lua table.insert() function - inserts element into array part of table
pub fn table_insert(vm: &mut VM) -> Result<LuaValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() < 3 {
        return Err("table.insert requires at least 2 arguments".to_string());
    }
    
    let table_val = &registers[1];
    let Some(_table) = table_val.as_table() else {
        return Err("table.insert expects a table as first argument".to_string());
    };
    
    // TODO: implement array insertion when Table supports array operations
    // For now, just return nil
    Ok(LuaValue::Nil)
}

/// Lua table.remove() function - removes element from array part of table
pub fn table_remove(vm: &mut VM) -> Result<LuaValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() < 2 {
        return Err("table.remove requires at least 1 argument".to_string());
    }
    
    let table_val = &registers[1];
    let Some(_table) = table_val.as_table() else {
        return Err("table.remove expects a table as first argument".to_string());
    };
    
    // TODO: implement array removal when Table supports array operations
    Ok(LuaValue::Nil)
}
