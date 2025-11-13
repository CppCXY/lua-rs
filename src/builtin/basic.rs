// Basic library functions (print, type, assert, etc.)

use crate::value::{LuaValue, MultiValue};
use crate::vm::VM;

/// Lua print() function - prints values to stdout
/// 
/// In our register-based VM, function arguments are passed in consecutive registers
/// starting from register 0. The number of arguments is determined by the calling convention.
/// For simplicity, we'll print all non-nil registers.
pub fn lua_print(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    let mut output = Vec::new();
    
    // Print all non-nil values from registers (skip first register which is the function itself)
    let mut first = true;
    for (i, value) in registers.iter().enumerate() {
        // Skip first register (function) and nil values at the end
        if i == 0 || matches!(value, LuaValue::Nil) {
            continue;
        }
        
        if !first {
            output.push("\t".to_string());
        }
        first = false;
        
        output.push(value.to_string_repr());
    }
    
    if !output.is_empty() {
        println!("{}", output.join(""));
    } else {
        println!();
    }
    
    Ok(MultiValue::empty())
}

/// Lua type() function - returns the type name of a value
pub fn lua_type(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    // First argument is in register 1 (register 0 is the function)
    if registers.len() <= 1 {
        return Err("type() requires 1 argument".to_string());
    }
    
    let value = &registers[1];
    let type_name = match value {
        LuaValue::Nil => "nil",
        LuaValue::Boolean(_) => "boolean",
        LuaValue::Integer(_) | LuaValue::Float(_) => "number",
        LuaValue::String(_) => "string",
        LuaValue::Table(_) => "table",
        LuaValue::Function(_) => "function",
        LuaValue::CFunction(_) => "function",
        LuaValue::Userdata(_) => "userdata",
    };
    
    let result_str = vm.create_builtin_string(type_name.to_string());
    Ok(MultiValue::single(LuaValue::String(result_str)))
}

/// Lua assert() function - raises error if condition is false
pub fn lua_assert(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 1 {
        return Err("assertion failed!".to_string());
    }
    
    let condition = &registers[1];
    if !condition.is_truthy() {
        let message = if registers.len() > 2 && !matches!(registers[2], LuaValue::Nil) {
            registers[2].to_string_repr()
        } else {
            "assertion failed!".to_string()
        };
        return Err(message);
    }
    
    // Return the first argument
    Ok(MultiValue::single(condition.clone()))
}

/// Lua tostring() function - converts value to string
pub fn lua_tostring(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 1 {
        let nil_str = vm.create_builtin_string("nil".to_string());
        return Ok(MultiValue::single(LuaValue::String(nil_str)));
    }
    
    let value = &registers[1];
    let result_str = vm.create_builtin_string(value.to_string_repr());
    Ok(MultiValue::single(LuaValue::String(result_str)))
}

/// Lua tonumber() function - converts value to number
pub fn lua_tonumber(vm: &mut VM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().ok_or("No call frame")?;
    let registers = &frame.registers;
    
    if registers.len() <= 1 {
        return Ok(MultiValue::single(LuaValue::Nil));
    }
    
    let value = &registers[1];
    
    let result = match value {
        LuaValue::Integer(i) => LuaValue::Integer(*i),
        LuaValue::Float(f) => LuaValue::Float(*f),
        LuaValue::String(s) => {
            // Try to parse as integer first
            if let Ok(i) = s.as_str().parse::<i64>() {
                LuaValue::Integer(i)
            } else if let Ok(f) = s.as_str().parse::<f64>() {
                LuaValue::Float(f)
            } else {
                LuaValue::Nil
            }
        }
        _ => LuaValue::Nil,
    };
    
    Ok(MultiValue::single(result))
}
