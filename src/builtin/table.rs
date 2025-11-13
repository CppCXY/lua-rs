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

/// Lua next() function - returns next key-value pair in table
/// Usage: k, v = next(table, key)
/// If key is nil, returns first key-value pair
/// If key is last key, returns nil
pub fn lua_next(vm: &mut VM) -> Result<LuaValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() < 2 {
        return Err("next requires at least 1 argument".to_string());
    }
    
    let table_val = &registers[1];
    let Some(table) = table_val.as_table() else {
        return Err("next expects a table as first argument".to_string());
    };
    
    let index_val = if registers.len() >= 3 {
        &registers[2]
    } else {
        &LuaValue::Nil
    };
    
    let table_ref = table.borrow();
    
    // Get all key-value pairs
    let pairs: Vec<_> = table_ref.iter_all().collect();
    
    if pairs.is_empty() {
        // Clear return values for empty table
        vm.return_values.clear();
        return Ok(LuaValue::Nil); // Empty table
    }
    
    // If index is nil, return first key-value pair
    if index_val.is_nil() {
        let (key, value) = &pairs[0];
        // Store both key and value in return_values
        vm.return_values = vec![key.clone(), value.clone()];
        return Ok(key.clone());
    }
    
    // Find current key position and return next
    for (i, (key, _value)) in pairs.iter().enumerate() {
        if key == index_val {
            if i + 1 < pairs.len() {
                let (next_key, next_value) = &pairs[i + 1];
                // Store both key and value in return_values
                vm.return_values = vec![next_key.clone(), next_value.clone()];
                return Ok(next_key.clone());
            } else {
                // No more keys - clear return values
                vm.return_values.clear();
                return Ok(LuaValue::Nil); // No more keys
            }
        }
    }
    
    Err("invalid key to 'next'".to_string())
}

/// Lua pairs() function - returns iterator for table
/// Usage: for k, v in pairs(table) do ... end
/// Returns: iterator_function, table, nil
pub fn lua_pairs(vm: &mut VM) -> Result<LuaValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() < 2 {
        return Err("pairs requires 1 argument".to_string());
    }
    
    let table_val = &registers[1];
    let Some(_table) = table_val.as_table() else {
        return Err("pairs expects a table as argument".to_string());
    };
    
    // Return the next function as a CFunction
    // TODO: This needs multi-return support to return (next, table, nil)
    // For now, return the table itself as a placeholder
    Ok(table_val.clone())
}

/// Lua ipairs() function - returns iterator for table array part
/// Usage: for i, v in ipairs(table) do ... end
/// Returns: iterator_function, table, 0
pub fn lua_ipairs(vm: &mut VM) -> Result<LuaValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() < 2 {
        return Err("ipairs requires 1 argument".to_string());
    }
    
    let table_val = &registers[1];
    let Some(_table) = table_val.as_table() else {
        return Err("ipairs expects a table as argument".to_string());
    };
    
    // Return the ipairs_next function
    // TODO: This needs multi-return support to return (ipairs_next, table, 0)
    // For now, return the table itself as a placeholder
    Ok(table_val.clone())
}
