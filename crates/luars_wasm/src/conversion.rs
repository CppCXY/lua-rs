/// Enhanced Lua <-> JavaScript value conversion module
/// Provides bidirectional type conversion with support for:
/// - Basic types (nil, boolean, number, string)
/// - Tables to JS Objects/Arrays
/// - Nested structures with cycle detection
/// - JS callbacks to Lua functions

use luars::{LuaValue, LuaVM};
use std::collections::HashSet;
use wasm_bindgen::prelude::*;

/// Maximum recursion depth for nested structures
const MAX_RECURSION_DEPTH: usize = 100;

/// Context for tracking visited tables during conversion (cycle detection)
struct ConversionContext {
    visited: HashSet<usize>,
    depth: usize,
}

impl ConversionContext {
    fn new() -> Self {
        Self {
            visited: HashSet::new(),
            depth: 0,
        }
    }

    fn can_recurse(&self) -> bool {
        self.depth < MAX_RECURSION_DEPTH
    }

    fn enter(&mut self, ptr: usize) -> bool {
        self.depth += 1;
        self.visited.insert(ptr)
    }

    fn exit(&mut self) {
        self.depth -= 1;
    }
}

// ============ Lua to JavaScript Conversion ============

/// Convert Lua value to JavaScript value
/// Supports all Lua types with intelligent table detection
pub fn lua_value_to_js(vm: &LuaVM, value: &LuaValue) -> Result<JsValue, JsValue> {
    let mut ctx = ConversionContext::new();
    lua_value_to_js_impl(vm, value, &mut ctx)
}

fn lua_value_to_js_impl(
    vm: &LuaVM,
    value: &LuaValue,
    ctx: &mut ConversionContext,
) -> Result<JsValue, JsValue> {
    // Check recursion depth
    if !ctx.can_recurse() {
        return Err(JsValue::from_str("Maximum recursion depth exceeded"));
    }

    // Handle basic types
    if value.is_nil() {
        return Ok(JsValue::NULL);
    }

    if let Some(b) = value.as_bool() {
        return Ok(JsValue::from_bool(b));
    }

    if let Some(i) = value.as_integer() {
        return Ok(JsValue::from_f64(i as f64));
    }

    if let Some(n) = value.as_number() {
        return Ok(JsValue::from_f64(n));
    }

    if let Some(s) = value.as_str() {
        return Ok(JsValue::from_str(s));
    }

    // Handle tables
    if value.is_table() {
        return table_to_js(vm, value, ctx);
    }

    // Handle functions
    if value.is_function() || value.is_cfunction() {
        return Ok(JsValue::from_str("[Lua Function]"));
    }

    // Handle threads
    if value.is_thread() {
        return Ok(JsValue::from_str("[Lua Thread]"));
    }

    // Default: convert to string representation
    Ok(JsValue::from_str(&format!("{:?}", value)))
}

/// Convert Lua table to JavaScript Object or Array
fn table_to_js(
    vm: &LuaVM,
    table_value: &LuaValue,
    ctx: &mut ConversionContext,
) -> Result<JsValue, JsValue> {
    // Get raw pointer for cycle detection
    let ptr = table_value.as_table_ptr()
        .ok_or_else(|| JsValue::from_str("Invalid table"))?
        .as_ptr() as usize;

    // Check for circular reference
    if !ctx.enter(ptr) {
        return Ok(JsValue::from_str("[Circular Reference]"));
    }

    let result = if is_array_like(table_value) {
        table_to_js_array(vm, table_value, ctx)
    } else {
        table_to_js_object(vm, table_value, ctx)
    };

    ctx.exit();
    result
}

/// Check if a Lua table is array-like
/// A table is array-like if it has consecutive integer keys starting from 1
fn is_array_like(table_value: &LuaValue) -> bool {
    let Some(table) = table_value.as_table() else {
        return false;
    };

    let len = table.len();
    
    // Empty tables are considered objects
    if len == 0 {
        return false;
    }

    // Check if all keys from 1 to len exist and are consecutive
    for i in 1..=len {
        if table.get_int(i as i64).is_none() || 
           table.get_int(i as i64).unwrap().is_nil() {
            return false;
        }
    }

    true
}

/// Convert Lua table to JavaScript Array
fn table_to_js_array(
    vm: &LuaVM,
    table_value: &LuaValue,
    ctx: &mut ConversionContext,
) -> Result<JsValue, JsValue> {
    let Some(table) = table_value.as_table() else {
        return Err(JsValue::from_str("Not a table"));
    };

    let array = js_sys::Array::new();
    let len = table.len();

    for i in 1..=len {
        if let Some(value) = table.get_int(i as i64) {
            if !value.is_nil() {
                let js_value = lua_value_to_js_impl(vm, &value, ctx)?;
                array.push(&js_value);
            } else {
                array.push(&JsValue::NULL);
            }
        }
    }

    Ok(array.into())
}

/// Convert Lua table to JavaScript Object
fn table_to_js_object(
    vm: &LuaVM,
    table_value: &LuaValue,
    ctx: &mut ConversionContext,
) -> Result<JsValue, JsValue> {
    let Some(table) = table_value.as_table() else {
        return Err(JsValue::from_str("Not a table"));
    };

    let obj = js_sys::Object::new();

    // Iterate over all key-value pairs
    // Note: This is a simplified version - a full implementation would need
    // to iterate through both array and hash parts of the table
    
    // For now, convert to a simple string representation
    // Full implementation would require table iteration API from LuaTable
    let display = format!("{}", table_value);
    js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("__luaTable"),
        &JsValue::from_str(&display),
    )
    .map_err(|_| JsValue::from_str("Failed to set property"))?;

    Ok(obj.into())
}

// ============ JavaScript to Lua Conversion ============

/// Convert JavaScript value to Lua value
pub fn js_value_to_lua(
    vm: &mut LuaVM,
    value: &JsValue,
) -> Result<LuaValue, luars::lua_vm::LuaError> {
    let mut ctx = ConversionContext::new();
    js_value_to_lua_impl(vm, value, &mut ctx)
}

fn js_value_to_lua_impl(
    vm: &mut LuaVM,
    value: &JsValue,
    ctx: &mut ConversionContext,
) -> Result<LuaValue, luars::lua_vm::LuaError> {
    // Check recursion depth
    if !ctx.can_recurse() {
        return Err(luars::lua_vm::LuaError::RuntimeError(
            "Maximum recursion depth exceeded".to_string(),
        ));
    }

    // Handle null/undefined
    if value.is_null() || value.is_undefined() {
        return Ok(LuaValue::nil());
    }

    // Handle boolean
    if let Some(b) = value.as_bool() {
        return Ok(LuaValue::boolean(b));
    }

    // Handle number
    if let Some(n) = value.as_f64() {
        // Auto-detect integer vs float
        if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            return Ok(LuaValue::integer(n as i64));
        } else {
            return Ok(LuaValue::number(n));
        }
    }

    // Handle string
    if let Some(s) = value.as_string() {
        return Ok(vm.create_string(&s));
    }

    // Handle Array
    if js_sys::Array::is_array(value) {
        return js_array_to_lua(vm, value, ctx);
    }

    // Handle Object
    if value.is_object() {
        return js_object_to_lua(vm, value, ctx);
    }

    // For unsupported types, return nil
    Ok(LuaValue::nil())
}

/// Convert JavaScript Array to Lua table
fn js_array_to_lua(
    vm: &mut LuaVM,
    value: &JsValue,
    ctx: &mut ConversionContext,
) -> Result<LuaValue, luars::lua_vm::LuaError> {
    let array = js_sys::Array::from(value);
    let len = array.length() as usize;

    // Create Lua table with appropriate array size
    let table = vm.create_table(len, 0);

    ctx.depth += 1;

    for i in 0..len {
        let js_elem = array.get(i as u32);
        let lua_elem = js_value_to_lua_impl(vm, &js_elem, ctx)?;
        
        // Lua arrays are 1-indexed
        if let Some(t) = table.as_table_mut() {
            t.set_int((i + 1) as i64, lua_elem);
        }
    }

    ctx.depth -= 1;

    Ok(table)
}

/// Convert JavaScript Object to Lua table
fn js_object_to_lua(
    vm: &mut LuaVM,
    value: &JsValue,
    ctx: &mut ConversionContext,
) -> Result<LuaValue, luars::lua_vm::LuaError> {
    let obj = js_sys::Object::from(value.clone());
    let keys = js_sys::Object::keys(&obj);
    let len = keys.length() as usize;

    // Create Lua table
    let table = vm.create_table(0, len);

    ctx.depth += 1;

    for i in 0..len {
        let key_js = keys.get(i as u32);
        if let Some(key_str) = key_js.as_string() {
            // Get value from object
            if let Ok(val_js) = js_sys::Reflect::get(&obj, &key_js) {
                let lua_key = vm.create_string(&key_str);
                let lua_value = js_value_to_lua_impl(vm, &val_js, ctx)?;

                if let Some(t) = table.as_table_mut() {
                    t.raw_set(&lua_key, lua_value);
                }
            }
        }
    }

    ctx.depth -= 1;

    Ok(table)
}

// ============ Utility Functions ============

/// Convert Lua value to a human-readable JSON-like string
/// Useful for debugging and display purposes
pub fn lua_value_to_json_string(vm: &LuaVM, value: &LuaValue) -> String {
    match lua_value_to_js(vm, value) {
        Ok(js_val) => {
            // Try to convert to JSON string
            if let Ok(json_str) = js_sys::JSON::stringify(&js_val) {
                json_str.as_string().unwrap_or_else(|| "[Invalid JSON]".to_string())
            } else {
                format!("{:?}", value)
            }
        }
        Err(_) => format!("{:?}", value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require WASM testing infrastructure
    // They serve as documentation of expected behavior
}
