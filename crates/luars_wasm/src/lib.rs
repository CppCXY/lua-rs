use luars::{LuaVM, LuaValue};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

// Set panic hook for better error messages in WASM
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Lua VM wrapper for WASM
#[wasm_bindgen]
pub struct LuaWasm {
    vm: Rc<RefCell<LuaVM>>,
}

#[wasm_bindgen]
impl LuaWasm {
    /// Create a new Lua VM instance
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<LuaWasm, JsValue> {
        let mut vm = LuaVM::new();
        vm.open_libs();

        Ok(LuaWasm {
            vm: Rc::new(RefCell::new(vm)),
        })
    }

    /// Execute Lua code and return the result as a string
    #[wasm_bindgen]
    pub fn execute(&self, code: &str) -> Result<String, JsValue> {
        let mut vm = self.vm.borrow_mut();

        match vm.execute_string(code) {
            Ok(result) => Ok(lua_value_to_string(&result)),
            Err(e) => Err(JsValue::from_str(&format!("Compilation error: {:?}", e))),
        }
    }

    /// Set a global variable in Lua
    #[wasm_bindgen(js_name = setGlobal)]
    pub fn set_global(&self, name: &str, value: JsValue) -> Result<(), JsValue> {
        let mut vm = self.vm.borrow_mut();
        let lua_value =
            js_value_to_lua(&value).map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        vm.set_global(name, lua_value);
        Ok(())
    }

    /// Get a global variable from Lua
    #[wasm_bindgen(js_name = getGlobal)]
    pub fn get_global(&self, name: &str) -> Result<JsValue, JsValue> {
        let mut vm = self.vm.borrow_mut();
        if let Some(lua_value) = vm.get_global(name) {
            lua_value_to_js(&lua_value)
        } else {
            Ok(JsValue::NULL)
        }
    }

    /// Evaluate a Lua expression and return the result as JsValue
    #[wasm_bindgen]
    pub fn eval(&self, expr: &str) -> Result<JsValue, JsValue> {
        let code = format!("return {}", expr);
        let mut vm = self.vm.borrow_mut();

        match vm.compile(&code) {
            Ok(chunk) => match vm.execute(Rc::new(chunk)) {
                Ok(value) => lua_value_to_js(&value),
                Err(e) => Err(JsValue::from_str(&format!("Runtime error: {:?}", e))),
            },
            Err(e) => Err(JsValue::from_str(&format!("Compilation error: {:?}", e))),
        }
    }

    /// Register a simple JavaScript callback that can be called from Lua
    /// Note: This is a simplified version - full JS callback support requires additional work
    #[wasm_bindgen(js_name = registerFunction)]
    pub fn register_function(
        &self,
        name: String,
        _callback: js_sys::Function,
    ) -> Result<(), JsValue> {
        // For now, we'll create a placeholder function
        // Full JS callback support would require using thread_local storage
        let code = format!(
            r#"
            function {}(...)
                error("JS callbacks not yet fully implemented - use setGlobal for values")
            end
        "#,
            name
        );

        let mut vm = self.vm.borrow_mut();
        let chunk = vm
            .compile(&code)
            .map_err(|e| JsValue::from_str(&format!("Failed to register function: {:?}", e)))?;

        vm.execute(Rc::new(chunk))
            .map_err(|e| JsValue::from_str(&format!("Failed to execute registration: {:?}", e)))?;

        Ok(())
    }
}

/// Convert Lua value to string representation
fn lua_value_to_string(value: &LuaValue) -> String {
    if value.is_nil() {
        "nil".to_string()
    } else if let Some(b) = value.as_bool() {
        b.to_string()
    } else if let Some(i) = value.as_integer() {
        i.to_string()
    } else if let Some(n) = value.as_number() {
        n.to_string()
    } else if value.is_string() {
        // Note: Without access to VM's object_pool, we can't resolve string content
        // This is a limitation - WASM wrapper should use VM methods for string conversion
        "[string]".to_string()
    } else if value.is_table() {
        "table".to_string()
    } else if value.is_function() || value.is_cfunction() {
        "function".to_string()
    } else if value.is_thread() {
        "thread".to_string()
    } else {
        format!("{:?}", value)
    }
}

/// Convert Lua value to JavaScript value
fn lua_value_to_js(value: &LuaValue) -> Result<JsValue, JsValue> {
    if value.is_nil() {
        Ok(JsValue::NULL)
    } else if let Some(b) = value.as_bool() {
        Ok(JsValue::from_bool(b))
    } else if let Some(i) = value.as_integer() {
        Ok(JsValue::from_f64(i as f64))
    } else if let Some(n) = value.as_number() {
        Ok(JsValue::from_f64(n))
    } else if value.is_string() {
        // Note: Without access to VM's object_pool, we can't resolve string content
        Ok(JsValue::from_str("[string]"))
    } else if value.is_table() {
        // For tables, we'll convert to a simple object representation
        Ok(JsValue::from_str("[Lua Table]"))
    } else if value.is_function() || value.is_cfunction() {
        Ok(JsValue::from_str("[Lua Function]"))
    } else if value.is_thread() {
        Ok(JsValue::from_str("[Lua Thread]"))
    } else {
        Ok(JsValue::from_str(&format!("{:?}", value)))
    }
}

/// Convert JavaScript value to Lua value
fn js_value_to_lua(value: &JsValue) -> Result<LuaValue, luars::lua_vm::LuaError> {
    if value.is_null() || value.is_undefined() {
        Ok(LuaValue::nil())
    } else if let Some(b) = value.as_bool() {
        Ok(LuaValue::boolean(b))
    } else if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            Ok(LuaValue::integer(n as i64))
        } else {
            Ok(LuaValue::number(n))
        }
    } else if let Some(_s) = value.as_string() {
        // String values - for now just return nil
        // We would need access to the VM's string pool to properly create strings
        Ok(LuaValue::nil())
    } else {
        // For complex objects
        Ok(LuaValue::nil())
    }
}
