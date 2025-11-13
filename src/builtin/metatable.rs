// Metatable-related builtin functions

use crate::value::{LuaValue, MultiValue};
use crate::vm::VM;

/// Lua getmetatable() function - returns the metatable of a value
pub fn lua_getmetatable(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 1 {
        return Ok(MultiValue::single(LuaValue::Nil));
    }
    
    let value = &registers[1];
    
    match value {
        LuaValue::Table(t) => {
            if let Some(mt) = t.borrow().get_metatable() {
                Ok(MultiValue::single(LuaValue::Table(mt)))
            } else {
                Ok(MultiValue::single(LuaValue::Nil))
            }
        }
        // TODO: Support metatables for other types (userdata, strings, etc.)
        _ => Ok(MultiValue::single(LuaValue::Nil))
    }
}

/// Lua setmetatable() function - sets the metatable of a table
pub fn lua_setmetatable(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 2 {
        return Err("setmetatable() requires 2 arguments".to_string());
    }
    
    let table = &registers[1];
    let metatable = &registers[2];
    
    // First argument must be a table
    if let LuaValue::Table(t) = table {
        // Check if current metatable has __metatable field (protected)
        if let Some(mt) = t.borrow().get_metatable() {
            let metatable_key = LuaValue::String(std::rc::Rc::new(crate::value::LuaString::new("__metatable".to_string())));
            if mt.borrow().raw_get(&metatable_key).is_some() {
                return Err("cannot change a protected metatable".to_string());
            }
        }
        
        // Set the new metatable
        match metatable {
            LuaValue::Nil => {
                t.borrow_mut().set_metatable(None);
            }
            LuaValue::Table(mt) => {
                t.borrow_mut().set_metatable(Some(mt.clone()));
            }
            _ => {
                return Err("setmetatable() second argument must be a table or nil".to_string());
            }
        }
        
        // Return the original table
        Ok(MultiValue::single(table.clone()))
    } else {
        Err("setmetatable() first argument must be a table".to_string())
    }
}

/// Lua rawget() function - gets value without metamethods
pub fn lua_rawget(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 2 {
        return Err("rawget() requires 2 arguments".to_string());
    }
    
    let table = &registers[1];
    let key = &registers[2];
    
    if let LuaValue::Table(t) = table {
        let value = t.borrow().raw_get(key).unwrap_or(LuaValue::Nil);
        Ok(MultiValue::single(value))
    } else {
        Err("rawget() first argument must be a table".to_string())
    }
}

/// Lua rawset() function - sets value without metamethods
pub fn lua_rawset(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 3 {
        return Err("rawset() requires 3 arguments".to_string());
    }
    
    let table = &registers[1];
    let key = &registers[2];
    let value = &registers[3];
    
    if let LuaValue::Table(t) = table {
        if matches!(key, LuaValue::Nil) {
            return Err("table index is nil".to_string());
        }
        
        t.borrow_mut().raw_set(key.clone(), value.clone());
        Ok(MultiValue::single(table.clone()))
    } else {
        Err("rawset() first argument must be a table".to_string())
    }
}

/// Lua rawlen() function - gets length without metamethods
pub fn lua_rawlen(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 1 {
        return Err("rawlen() requires 1 argument".to_string());
    }
    
    let value = &registers[1];
    
    let len = match value {
        LuaValue::Table(t) => t.borrow().len() as i64,
        LuaValue::String(s) => s.as_str().len() as i64,
        _ => {
            return Err("rawlen() argument must be a table or string".to_string());
        }
    };
    
    Ok(MultiValue::single(LuaValue::Integer(len)))
}
