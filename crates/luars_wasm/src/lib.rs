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
            Ok(results) => {
                let result = results.into_iter().next().unwrap_or(luars::LuaValue::nil());
                Ok(vm.value_to_string_raw(&result))
            },
            Err(e) => Err(JsValue::from_str(&format!("Compilation error: {:?}", e))),
        }
    }

    /// Set a global variable in Lua
    #[wasm_bindgen(js_name = setGlobal)]
    pub fn set_global(&self, name: &str, value: JsValue) -> Result<(), JsValue> {
        let mut vm = self.vm.borrow_mut();
        let lua_value =
            js_value_to_lua(&mut vm, &value).map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        vm.set_global(name, lua_value);
        Ok(())
    }

    /// Get a global variable from Lua
    #[wasm_bindgen(js_name = getGlobal)]
    pub fn get_global(&self, name: &str) -> Result<JsValue, JsValue> {
        let mut vm = self.vm.borrow_mut();
        if let Some(lua_value) = vm.get_global(name) {
            lua_value_to_js(&vm, &lua_value)
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
                Ok(results) => {
                    let value = results.into_iter().next().unwrap_or(luars::LuaValue::nil());
                    lua_value_to_js(&vm, &value)
                },
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

/// Convert Lua value to JavaScript value
fn lua_value_to_js(vm: &LuaVM, value: &LuaValue) -> Result<JsValue, JsValue> {
    if value.is_nil() {
        Ok(JsValue::NULL)
    } else if let Some(b) = value.as_bool() {
        Ok(JsValue::from_bool(b))
    } else if let Some(i) = value.as_integer() {
        Ok(JsValue::from_f64(i as f64))
    } else if let Some(n) = value.as_number() {
        Ok(JsValue::from_f64(n))
    } else if value.is_string() {
        // Use VM's value_as_string to properly resolve the string
        if let Some(s) = vm.value_as_string(value) {
            Ok(JsValue::from_str(&s))
        } else {
            Ok(JsValue::from_str("[invalid string]"))
        }
    } else if value.is_table() {
        // For tables, we'll convert to a simple object representation
        Ok(JsValue::from_str(&vm.value_to_string_raw(value)))
    } else if value.is_function() || value.is_cfunction() {
        Ok(JsValue::from_str("[Lua Function]"))
    } else if value.is_thread() {
        Ok(JsValue::from_str("[Lua Thread]"))
    } else {
        Ok(JsValue::from_str(&format!("{:?}", value)))
    }
}

/// Convert JavaScript value to Lua value
fn js_value_to_lua(vm: &mut LuaVM, value: &JsValue) -> Result<LuaValue, luars::lua_vm::LuaError> {
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
    } else if let Some(s) = value.as_string() {
        // Use VM's create_string to properly create a string in the object pool
        Ok(vm.create_string(&s))
    } else {
        // For complex objects, return nil
        Ok(LuaValue::nil())
    }
}
