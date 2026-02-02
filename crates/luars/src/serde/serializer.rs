/// Serializer for converting Lua values to serde_json::Value
///
/// This handles the conversion from Lua's dynamic type system to JSON:
/// - Lua nil -> JSON null
/// - Lua boolean -> JSON boolean
/// - Lua number -> JSON number
/// - Lua string -> JSON string
/// - Lua table (array-like) -> JSON array
/// - Lua table (map-like) -> JSON object
/// - Other types -> error or special handling
use crate::lua_value::LuaValue;
use serde_json::{Map, Number, Value as JsonValue};
use std::collections::HashSet;

/// Convert a Lua value to a serde_json::Value
pub fn to_value(lua_value: &LuaValue) -> Result<JsonValue, String> {
    let mut visited = HashSet::new();
    to_value_internal(lua_value, &mut visited)
}

/// Convert a Lua value to a JSON string
pub fn to_string(lua_value: &LuaValue, pretty: bool) -> Result<String, String> {
    let json_value = to_value(lua_value)?;

    if pretty {
        serde_json::to_string_pretty(&json_value)
            .map_err(|e| format!("Failed to serialize to JSON: {}", e))
    } else {
        serde_json::to_string(&json_value)
            .map_err(|e| format!("Failed to serialize to JSON: {}", e))
    }
}

fn to_value_internal(
    lua_value: &LuaValue,
    visited: &mut HashSet<usize>,
) -> Result<JsonValue, String> {
    match lua_value {
        // Simple types
        _ if lua_value.is_nil() => Ok(JsonValue::Null),

        _ if lua_value.is_boolean() => Ok(JsonValue::Bool(lua_value.as_bool().unwrap())),

        _ if lua_value.is_number() => {
            let num = lua_value.as_number().unwrap();
            // Check if it's an integer
            if num.fract() == 0.0 && num.is_finite() {
                Ok(JsonValue::Number(Number::from(num as i64)))
            } else {
                // Use float
                Number::from_f64(num)
                    .map(JsonValue::Number)
                    .ok_or_else(|| format!("Invalid number: {}", num))
            }
        }

        _ if lua_value.is_string() => lua_value
            .as_str()
            .map(|s| JsonValue::String(s.to_string()))
            .ok_or_else(|| "Failed to get string value".to_string()),

        _ if lua_value.is_table() => {
            // Get table pointer for cycle detection
            let table_ptr = lua_value
                .as_table_ptr()
                .ok_or_else(|| "Failed to get table pointer".to_string())?;
            let ptr_addr = table_ptr.as_ptr() as usize;

            // Check for circular reference
            if visited.contains(&ptr_addr) {
                return Err("Circular reference detected in table".to_string());
            }
            visited.insert(ptr_addr);

            let table_ref = table_ptr.as_ref();

            // Determine if table is array-like or object-like
            // A table is array-like if:
            // 1. All keys are consecutive integers starting from 1
            // 2. No other keys exist

            let is_array = is_array_like(&table_ref.data);

            let result = if is_array {
                table_to_json_array(&table_ref.data, visited)
            } else {
                table_to_json_object(&table_ref.data, visited)
            };

            visited.remove(&ptr_addr);
            result
        }

        _ if lua_value.is_function() => Err("Cannot serialize Lua function to JSON".to_string()),

        _ if lua_value.is_thread() => Err("Cannot serialize Lua thread to JSON".to_string()),

        _ if lua_value.is_userdata() => Err("Cannot serialize Lua userdata to JSON".to_string()),

        _ => Err(format!(
            "Unsupported Lua type for JSON serialization: {:?}",
            lua_value
        )),
    }
}

fn is_array_like(table: &crate::lua_value::LuaTable) -> bool {
    // Get all keys
    let keys = table.iter_keys();

    if keys.is_empty() {
        return true; // Empty table is treated as array
    }

    // Check if all keys are consecutive integers starting from 1
    let mut int_keys: Vec<i64> = Vec::new();

    for key in &keys {
        if let Some(num) = key.as_number() {
            if num > 0.0 && num.fract() == 0.0 {
                int_keys.push(num as i64);
            } else {
                return false; // Non-positive or non-integer number key
            }
        } else {
            return false; // Non-number key
        }
    }

    // Sort and check for consecutive sequence starting from 1
    int_keys.sort_unstable();

    if int_keys.is_empty() || int_keys[0] != 1 {
        return false;
    }

    for i in 0..int_keys.len() - 1 {
        if int_keys[i + 1] != int_keys[i] + 1 {
            return false; // Gap in sequence
        }
    }

    true
}

fn table_to_json_array(
    table: &crate::lua_value::LuaTable,
    visited: &mut HashSet<usize>,
) -> Result<JsonValue, String> {
    let mut array = Vec::new();

    // Get length (consecutive integer keys from 1)
    let len = table.len();

    for i in 1..=len {
        let key = LuaValue::number(i as f64);
        if let Some(value) = table.raw_get(&key) {
            let json_value = to_value_internal(&value, visited)?;
            array.push(json_value);
        } else {
            // Gap in array - this shouldn't happen if is_array_like is correct
            array.push(JsonValue::Null);
        }
    }

    Ok(JsonValue::Array(array))
}

fn table_to_json_object(
    table: &crate::lua_value::LuaTable,
    visited: &mut HashSet<usize>,
) -> Result<JsonValue, String> {
    let mut object = Map::new();

    for key in table.iter_keys() {
        // Convert key to string
        let key_str = if let Some(s) = key.as_str() {
            s.to_string()
        } else if let Some(num) = key.as_number() {
            if num.fract() == 0.0 {
                format!("{}", num as i64)
            } else {
                format!("{}", num)
            }
        } else if key.is_boolean() {
            format!("{}", key.as_bool().unwrap())
        } else {
            continue; // Skip non-serializable keys
        };

        // Get value
        if let Some(value) = table.raw_get(&key) {
            let json_value = to_value_internal(&value, visited)?;
            object.insert(key_str, json_value);
        }
    }

    Ok(JsonValue::Object(object))
}
