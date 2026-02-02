/// Deserializer for converting serde_json::Value to Lua values
///
/// This handles the conversion from JSON to Lua's dynamic type system:
/// - JSON null -> Lua nil
/// - JSON boolean -> Lua boolean
/// - JSON number -> Lua number
/// - JSON string -> Lua string
/// - JSON array -> Lua table (array-like)
/// - JSON object -> Lua table (map-like)
use crate::lua_value::LuaValue;
use crate::lua_vm::LuaVM;
use serde_json::Value as JsonValue;

/// Convert a serde_json::Value to a Lua value
pub fn from_value(json_value: &JsonValue, vm: &mut LuaVM) -> Result<LuaValue, String> {
    match json_value {
        JsonValue::Null => Ok(LuaValue::nil()),

        JsonValue::Bool(b) => Ok(LuaValue::boolean(*b)),

        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::number(i as f64))
            } else if let Some(u) = n.as_u64() {
                Ok(LuaValue::number(u as f64))
            } else if let Some(f) = n.as_f64() {
                Ok(LuaValue::number(f))
            } else {
                Err("Invalid JSON number".to_string())
            }
        }

        JsonValue::String(s) => vm
            .create_string(s)
            .map_err(|e| format!("Failed to create string: {}", e)),

        JsonValue::Array(arr) => json_array_to_lua_table(arr, vm),

        JsonValue::Object(obj) => json_object_to_lua_table(obj, vm),
    }
}

/// Convert a JSON string to a Lua value
pub fn from_str(json_str: &str, vm: &mut LuaVM) -> Result<LuaValue, String> {
    let json_value: JsonValue =
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    from_value(&json_value, vm)
}

fn json_array_to_lua_table(arr: &[JsonValue], vm: &mut LuaVM) -> Result<LuaValue, String> {
    // Create table with array size hint
    let table = vm
        .create_table(arr.len(), 0)
        .map_err(|e| format!("Failed to create table: {}", e))?;

    // Fill table with array elements (1-indexed)
    for (i, item) in arr.iter().enumerate() {
        let value = from_value(item, vm)?;
        let key = LuaValue::number((i + 1) as f64);
        vm.raw_set(&table, key, value);
    }

    Ok(table)
}

fn json_object_to_lua_table(
    obj: &serde_json::Map<String, JsonValue>,
    vm: &mut LuaVM,
) -> Result<LuaValue, String> {
    // Create table with hash size hint
    let table = vm
        .create_table(0, obj.len())
        .map_err(|e| format!("Failed to create table: {}", e))?;

    // Fill table with object entries
    for (key_str, value_json) in obj {
        let key = vm
            .create_string(key_str)
            .map_err(|e| format!("Failed to create key string: {}", e))?;
        let value = from_value(value_json, vm)?;
        vm.raw_set(&table, key, value);
    }

    Ok(table)
}
