/// Bidirectional Lua ↔ JavaScript value conversion.
///
/// Supports:
/// - Primitives: nil ↔ null, boolean, integer/float ↔ number, string
/// - Binary strings ↔ Uint8Array
/// - Tables ↔ Array / Object (with cycle detection & depth guard)
/// - Functions → metadata objects
/// - UserData / Thread → metadata objects
use luars::{LuaVM, LuaValue};
use std::collections::HashSet;
use wasm_bindgen::prelude::*;

const MAX_DEPTH: usize = 64;

// ── Conversion context (cycle + depth guard) ────────────────────────────

struct Ctx {
    visited: HashSet<usize>,
    depth: usize,
}

impl Ctx {
    fn new() -> Self {
        Self {
            visited: HashSet::new(),
            depth: 0,
        }
    }
    fn enter_table(&mut self, ptr: usize) -> bool {
        if self.depth >= MAX_DEPTH {
            return false;
        }
        self.depth += 1;
        self.visited.insert(ptr)
    }
    fn exit(&mut self) {
        self.depth -= 1;
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Lua → JS
// ════════════════════════════════════════════════════════════════════════

pub fn lua_to_js(vm: &LuaVM, value: &LuaValue) -> JsValue {
    let mut ctx = Ctx::new();
    lua_to_js_inner(vm, value, &mut ctx)
}

fn lua_to_js_inner(vm: &LuaVM, value: &LuaValue, ctx: &mut Ctx) -> JsValue {
    if value.is_nil() {
        return JsValue::NULL;
    }
    if let Some(b) = value.as_bool() {
        return JsValue::from_bool(b);
    }
    if let Some(i) = value.as_integer() {
        return JsValue::from_f64(i as f64);
    }
    if let Some(n) = value.as_number() {
        return JsValue::from_f64(n);
    }
    if let Some(s) = value.as_str() {
        return JsValue::from_str(s);
    }
    if let Some(b) = value.as_binary() {
        let arr = js_sys::Uint8Array::new_with_length(b.len() as u32);
        arr.copy_from(b);
        return arr.into();
    }
    if value.is_table() {
        return table_to_js(vm, value, ctx);
    }
    if value.is_callable() {
        return function_to_js_meta(value);
    }
    if value.is_thread() {
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &"__type".into(), &"LuaThread".into());
        return obj.into();
    }
    if value.is_userdata() {
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &"__type".into(), &"LuaUserdata".into());
        return obj.into();
    }
    JsValue::NULL
}

fn table_to_js(vm: &LuaVM, table_value: &LuaValue, ctx: &mut Ctx) -> JsValue {
    let Some(table) = table_value.as_table() else {
        return JsValue::NULL;
    };
    let ptr = table_value
        .as_table_ptr()
        .map(|p| p.as_ptr() as usize)
        .unwrap_or(0);
    if !ctx.enter_table(ptr) {
        return JsValue::from_str("[Circular]");
    }
    let result = if table.is_array() && table.len() > 0 {
        table_to_js_array(vm, table_value, ctx)
    } else {
        table_to_js_object(vm, table_value, ctx)
    };
    ctx.exit();
    result
}

fn table_to_js_array(vm: &LuaVM, table_value: &LuaValue, ctx: &mut Ctx) -> JsValue {
    let Some(table) = table_value.as_table() else {
        return JsValue::NULL;
    };
    let len = table.len();
    let arr = js_sys::Array::new_with_length(len as u32);
    for i in 1..=len {
        let v = table.raw_geti(i as i64);
        let js_v = match &v {
            Some(val) if !val.is_nil() => lua_to_js_inner(vm, val, ctx),
            _ => JsValue::NULL,
        };
        arr.set((i - 1) as u32, js_v);
    }
    arr.into()
}

fn table_to_js_object(vm: &LuaVM, table_value: &LuaValue, ctx: &mut Ctx) -> JsValue {
    let Some(table) = table_value.as_table() else {
        return JsValue::NULL;
    };
    let obj = js_sys::Object::new();
    for (k, v) in table.iter_all() {
        let key_str = if let Some(s) = k.as_str() {
            s.to_string()
        } else if let Some(i) = k.as_integer() {
            i.to_string()
        } else if let Some(n) = k.as_number() {
            n.to_string()
        } else if let Some(b) = k.as_bool() {
            b.to_string()
        } else {
            continue;
        };
        let js_v = lua_to_js_inner(vm, &v, ctx);
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(&key_str), &js_v);
    }
    obj.into()
}

fn function_to_js_meta(value: &LuaValue) -> JsValue {
    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&obj, &"__type".into(), &"LuaFunction".into());
    let kind = if value.is_lua_function() {
        "lua"
    } else {
        "native"
    };
    let _ = js_sys::Reflect::set(&obj, &"kind".into(), &kind.into());
    obj.into()
}

// ════════════════════════════════════════════════════════════════════════
//  JS → Lua
// ════════════════════════════════════════════════════════════════════════

pub fn js_to_lua(vm: &mut LuaVM, value: &JsValue) -> Result<LuaValue, luars::lua_vm::LuaError> {
    let mut ctx = Ctx::new();
    js_to_lua_inner(vm, value, &mut ctx)
}

fn js_to_lua_inner(
    vm: &mut LuaVM,
    value: &JsValue,
    ctx: &mut Ctx,
) -> Result<LuaValue, luars::lua_vm::LuaError> {
    if ctx.depth >= MAX_DEPTH {
        return Ok(LuaValue::nil());
    }
    if value.is_null() || value.is_undefined() {
        return Ok(LuaValue::nil());
    }
    if let Some(b) = value.as_bool() {
        return Ok(LuaValue::boolean(b));
    }
    if let Some(n) = value.as_f64() {
        return Ok(js_number_to_lua(n));
    }
    if let Some(s) = value.as_string() {
        return vm.create_string(&s);
    }
    // Uint8Array → binary string
    if value.is_instance_of::<js_sys::Uint8Array>() {
        let arr = js_sys::Uint8Array::from(value.clone());
        return vm.create_binary(arr.to_vec());
    }
    // Array → sequence table
    if js_sys::Array::is_array(value) {
        return js_array_to_lua(vm, value, ctx);
    }
    // Object → hash table
    if value.is_object() {
        return js_object_to_lua(vm, value, ctx);
    }
    Ok(LuaValue::nil())
}

pub fn js_number_to_lua(n: f64) -> LuaValue {
    if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
        LuaValue::integer(n as i64)
    } else {
        LuaValue::number(n)
    }
}

fn js_array_to_lua(
    vm: &mut LuaVM,
    value: &JsValue,
    ctx: &mut Ctx,
) -> Result<LuaValue, luars::lua_vm::LuaError> {
    let array = js_sys::Array::from(value);
    let len = array.length() as usize;
    let table = vm.create_table(len, 0)?;
    ctx.depth += 1;
    for i in 0..len {
        let elem = array.get(i as u32);
        let lua_elem = js_to_lua_inner(vm, &elem, ctx)?;
        vm.raw_set(&table, LuaValue::integer((i + 1) as i64), lua_elem);
    }
    ctx.depth -= 1;
    Ok(table)
}

fn js_object_to_lua(
    vm: &mut LuaVM,
    value: &JsValue,
    ctx: &mut Ctx,
) -> Result<LuaValue, luars::lua_vm::LuaError> {
    let obj = js_sys::Object::from(value.clone());
    let keys = js_sys::Object::keys(&obj);
    let len = keys.length() as usize;
    let table = vm.create_table(0, len)?;
    ctx.depth += 1;
    for i in 0..len {
        let key_js = keys.get(i as u32);
        if let Some(key_str) = key_js.as_string() {
            if let Ok(val_js) = js_sys::Reflect::get(&obj, &key_js) {
                let lua_key = vm.create_string(&key_str)?;
                let lua_val = js_to_lua_inner(vm, &val_js, ctx)?;
                vm.raw_set(&table, lua_key, lua_val);
            }
        }
    }
    ctx.depth -= 1;
    Ok(table)
}

// ════════════════════════════════════════════════════════════════════════
//  Lightweight (no-VM) helpers for registered callbacks
// ════════════════════════════════════════════════════════════════════════

pub fn lua_to_js_basic(value: &LuaValue) -> JsValue {
    if value.is_nil() {
        JsValue::NULL
    } else if let Some(b) = value.as_bool() {
        JsValue::from_bool(b)
    } else if let Some(i) = value.as_integer() {
        JsValue::from_f64(i as f64)
    } else if let Some(n) = value.as_number() {
        JsValue::from_f64(n)
    } else if let Some(s) = value.as_str() {
        JsValue::from_str(s)
    } else {
        JsValue::NULL
    }
}

pub fn js_to_lua_basic(
    state: &mut luars::lua_vm::LuaState,
    value: &JsValue,
) -> Result<LuaValue, String> {
    if value.is_null() || value.is_undefined() {
        Ok(LuaValue::nil())
    } else if let Some(b) = value.as_bool() {
        Ok(LuaValue::boolean(b))
    } else if let Some(n) = value.as_f64() {
        Ok(js_number_to_lua(n))
    } else if let Some(s) = value.as_string() {
        state
            .create_string(&s)
            .map_err(|e| format!("create_string: {:?}", e))
    } else {
        Ok(LuaValue::nil())
    }
}
