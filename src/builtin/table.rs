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
/// Returns: next, table, nil
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
    
    // Return (next, table, nil) as three values
    // First return value is the next function
    let next_func = vm.get_global("next").unwrap_or(LuaValue::Nil);
    
    // Store additional return values in return_values buffer
    vm.return_values = vec![next_func.clone(), table_val.clone(), LuaValue::Nil];
    
    // Return first value (next function)
    Ok(next_func)
}

/// Iterator function for ipairs - returns next index and value
fn ipairs_next(vm: &mut VM) -> Result<LuaValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() < 3 {
        return Err("ipairs iterator requires 2 arguments".to_string());
    }
    
    let table_val = &registers[1];
    let index_val = &registers[2];
    
    let Some(table) = table_val.as_table() else {
        return Err("ipairs iterator expects a table".to_string());
    };
    
    // Get current index (should be integer)
    let current_index = if let Some(n) = index_val.as_number() {
        n as i64
    } else {
        0
    };
    
    let next_index = current_index + 1;
    let next_index_val = LuaValue::integer(next_index);
    
    // Try to get value at next index
    let table_ref = table.borrow();
    if let Some(value) = table_ref.get(&next_index_val) {
        if !value.is_nil() {
            // Return (index, value)
            vm.return_values = vec![next_index_val.clone(), value.clone()];
            return Ok(next_index_val);
        }
    }
    
    // No more values - return nil
    vm.return_values.clear();
    Ok(LuaValue::Nil)
}

/// Lua ipairs() function - returns iterator for table array part
/// Usage: for i, v in ipairs(table) do ... end
/// Returns: ipairs_next, table, 0
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
    
    // Return (ipairs_next, table, 0) as three values
    let iter_func = LuaValue::cfunction(ipairs_next);
    
    // Store additional return values
    vm.return_values = vec![iter_func.clone(), table_val.clone(), LuaValue::integer(0)];
    
    // Return first value (iterator function)
    Ok(iter_func)
}
