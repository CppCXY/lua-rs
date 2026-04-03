//! LuaValue → VariableProto serialization.

use luars::{LuaValue, LuaValueKind};

use crate::proto::{ValueType, Variable};

/// Convert a LuaValue to its EmmyLua type constant.
fn lua_type_to_value_type(v: &LuaValue) -> ValueType {
    match v.kind() {
        LuaValueKind::Nil => ValueType::TNIL,
        LuaValueKind::Boolean => ValueType::TBOOLEAN,
        LuaValueKind::Integer => ValueType::TNUMBER,
        LuaValueKind::Float => ValueType::TNUMBER,
        LuaValueKind::String => ValueType::TSTRING,
        LuaValueKind::Table => ValueType::TTABLE,
        LuaValueKind::Function => ValueType::TFUNCTION,
        LuaValueKind::CFunction => ValueType::TFUNCTION,
        LuaValueKind::CClosure => ValueType::TFUNCTION,
        LuaValueKind::RClosure => ValueType::TFUNCTION,
        LuaValueKind::Userdata => ValueType::TUSERDATA,
        LuaValueKind::Thread => ValueType::TTHREAD,
    }
}

/// Build a VariableProto from a name and LuaValue.
/// `depth` controls how many levels of children to expand for tables.
/// `cache_id` is a mutable counter for assigning unique cache IDs.
pub fn make_variable(name: &str, value: &LuaValue, depth: i32, cache_id: &mut i32) -> Variable {
    let value_type = lua_type_to_value_type(value);
    let type_name = value.type_name().to_string();
    let value_str = format!("{value}");

    let mut children = Vec::new();

    // Expand table children if depth > 0
    if depth > 0
        && let Some(table) = value.as_table()
    {
        let pairs = table.iter_all();
        for (k, v) in &pairs {
            let child_name = if let Some(s) = k.as_str() {
                s.to_string()
            } else if let Some(i) = k.as_integer() {
                format!("[{i}]")
            } else {
                format!("{k}")
            };
            *cache_id += 1;
            children.push(make_variable(&child_name, v, depth - 1, cache_id));
        }
    }

    *cache_id += 1;
    Variable {
        name: name.to_string(),
        name_type: ValueType::TSTRING, // name is always a string
        value: value_str,
        value_type,
        value_type_name: type_name,
        cache_id: *cache_id,
        children: Some(children),
    }
}
