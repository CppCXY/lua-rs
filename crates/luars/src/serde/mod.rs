/// Serde serialization support for Lua values
///
/// This module provides serialization/deserialization for Lua values using serde.
/// It's designed to be used through LuaVM/LuaState methods rather than implementing
/// Serialize/Deserialize traits directly on Lua types.

#[cfg(feature = "serde")]
mod serializer;

#[cfg(feature = "serde")]
mod deserializer;

#[cfg(feature = "serde")]
pub use serializer::{to_string as serialize_to_json_string, to_value as serialize_to_json};

#[cfg(feature = "serde")]
pub use deserializer::{
    from_str as deserialize_from_json_str, from_value as deserialize_from_json,
};

#[cfg(feature = "serde")]
use crate::lua_value::LuaValue;

/// Convert a Lua value to a serde_json::Value
#[cfg(feature = "serde")]
pub fn lua_to_json(lua_value: &LuaValue) -> Result<serde_json::Value, String> {
    serialize_to_json(lua_value)
}

/// Convert a Lua value to a JSON string
#[cfg(feature = "serde")]
pub fn lua_to_json_string(lua_value: &LuaValue, pretty: bool) -> Result<String, String> {
    serialize_to_json_string(lua_value, pretty)
}

/// Convert a serde_json::Value to a Lua value
#[cfg(feature = "serde")]
pub fn json_to_lua(
    json_value: &serde_json::Value,
    vm: &mut crate::lua_vm::LuaVM,
) -> Result<LuaValue, String> {
    deserialize_from_json(json_value, vm)
}

/// Convert a JSON string to a Lua value
#[cfg(feature = "serde")]
pub fn json_string_to_lua(
    json_str: &str,
    vm: &mut crate::lua_vm::LuaVM,
) -> Result<LuaValue, String> {
    deserialize_from_json_str(json_str, vm)
}
