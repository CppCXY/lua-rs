//! LuaValue → VariableProto serialization.

use luars::LuaValue;

use crate::proto::VariableProto;

// Lua type constants matching EmmyLua protocol
const LUA_TNIL: i32 = 0;
const LUA_TBOOLEAN: i32 = 1;
const LUA_TNUMBER: i32 = 3;
const LUA_TSTRING: i32 = 4;
const LUA_TTABLE: i32 = 5;
const LUA_TFUNCTION: i32 = 6;
const LUA_TUSERDATA: i32 = 7;
const LUA_TTHREAD: i32 = 8;

/// Convert a LuaValue to its EmmyLua type constant.
fn lua_type_id(v: &LuaValue) -> i32 {
    if v.is_nil() {
        LUA_TNIL
    } else if v.is_boolean() {
        LUA_TBOOLEAN
    } else if v.is_number() {
        LUA_TNUMBER
    } else if v.is_string() {
        LUA_TSTRING
    } else if v.is_table() {
        LUA_TTABLE
    } else if v.is_function() {
        LUA_TFUNCTION
    } else if v.is_userdata() {
        LUA_TUSERDATA
    } else if v.is_thread() {
        LUA_TTHREAD
    } else {
        LUA_TNIL
    }
}

/// Convert a LuaValue to its display string.
fn value_to_string(v: &LuaValue) -> String {
    if v.is_nil() {
        "nil".to_string()
    } else if let Some(b) = v.as_boolean() {
        if b {
            "true".to_string()
        } else {
            "false".to_string()
        }
    } else if let Some(i) = v.as_integer() {
        i.to_string()
    } else if let Some(n) = v.as_number() {
        format!("{n}")
    } else if let Some(s) = v.as_str() {
        format!("\"{s}\"")
    } else {
        // table, function, userdata, thread → use Display
        format!("{v}")
    }
}

/// Build a VariableProto from a name and LuaValue.
/// `depth` controls how many levels of children to expand for tables.
/// `cache_id` is a mutable counter for assigning unique cache IDs.
pub fn make_variable(
    name: &str,
    value: &LuaValue,
    depth: i32,
    cache_id: &mut i32,
) -> VariableProto {
    let type_id = lua_type_id(value);
    let type_name = value.type_name().to_string();
    let value_str = value_to_string(value);

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
    VariableProto {
        name: name.to_string(),
        name_type: LUA_TSTRING, // name is always a string
        value: value_str,
        value_type: type_id,
        value_type_name: type_name,
        cache_id: *cache_id,
        children,
    }
}
