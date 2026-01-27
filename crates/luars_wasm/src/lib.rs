use luars::{lua_vm::SafeOption, stdlib, LuaVM};
use wasm_bindgen::prelude::*;

mod conversion;
use conversion::{js_value_to_lua, lua_value_to_js, lua_value_to_json_string};

// Set panic hook for better error messages in WASM
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// simple impl, need more work
/// Lua VM wrapper for WASM
#[wasm_bindgen]
pub struct LuaWasm {
    vm: Box<LuaVM>,
}

#[wasm_bindgen]
impl LuaWasm {
    /// Create a new Lua VM instance
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<LuaWasm, JsValue> {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.open_stdlib(stdlib::Stdlib::All).unwrap();

        Ok(LuaWasm { vm })
    }

    /// Execute Lua code and return the result as a string
    #[wasm_bindgen]
    pub fn execute(&mut self, code: &str) -> Result<String, JsValue> {
        match self.vm.execute_string(code) {
            Ok(results) => {
                let result = results.into_iter().next().unwrap_or(luars::LuaValue::nil());
                Ok(format!("{}", result))
            }
            Err(e) => Err(JsValue::from_str(&format!("Compilation error: {:?}", e))),
        }
    }

    /// Set a global variable in Lua
    #[wasm_bindgen(js_name = setGlobal)]
    pub fn set_global(&mut self, name: &str, value: JsValue) -> Result<(), JsValue> {
        let lua_value = js_value_to_lua(&mut *self.vm, &value)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.vm.set_global(name, lua_value).unwrap();
        Ok(())
    }

    /// Get a global variable from Lua
    #[wasm_bindgen(js_name = getGlobal)]
    pub fn get_global(&mut self, name: &str) -> Result<JsValue, JsValue> {
        if let Some(lua_value) = self.vm.get_global(name).unwrap() {
            lua_value_to_js(&*self.vm, &lua_value)
        } else {
            Ok(JsValue::NULL)
        }
    }

    /// Evaluate a Lua expression and return the result as JsValue
    #[wasm_bindgen]
    pub fn eval(&mut self, expr: &str) -> Result<JsValue, JsValue> {
        let code = format!("return {}", expr);

        match self.vm.compile(&code) {
            Ok(chunk) => match self.vm.execute(std::rc::Rc::new(chunk)) {
                Ok(results) => {
                    let value = results.into_iter().next().unwrap_or(luars::LuaValue::nil());
                    lua_value_to_js(&*self.vm, &value)
                }
                Err(e) => Err(JsValue::from_str(&format!("Runtime error: {:?}", e))),
            },
            Err(e) => Err(JsValue::from_str(&format!("Compilation error: {:?}", e))),
        }
    }

    /// Evaluate Lua code and return the result as JSON string
    /// Useful for getting structured data from Lua tables
    #[wasm_bindgen(js_name = evalJson)]
    pub fn eval_json(&mut self, expr: &str) -> Result<String, JsValue> {
        let code = format!("return {}", expr);

        match self.vm.compile(&code) {
            Ok(chunk) => match self.vm.execute(std::rc::Rc::new(chunk)) {
                Ok(results) => {
                    let value = results.into_iter().next().unwrap_or(luars::LuaValue::nil());
                    Ok(lua_value_to_json_string(&*self.vm, &value))
                }
                Err(e) => Err(JsValue::from_str(&format!("Runtime error: {:?}", e))),
            },
            Err(e) => Err(JsValue::from_str(&format!("Compilation error: {:?}", e))),
        }
    }

    /// Register a simple JavaScript callback that can be called from Lua
    /// Note: This is a simplified version - full JS callback support requires additional work
    #[wasm_bindgen(js_name = registerFunction)]
    pub fn register_function(
        &mut self,
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

        let chunk = self
            .vm
            .compile(&code)
            .map_err(|e| JsValue::from_str(&format!("Failed to register function: {:?}", e)))?;

        self.vm
            .execute(std::rc::Rc::new(chunk))
            .map_err(|e| JsValue::from_str(&format!("Failed to execute registration: {:?}", e)))?;

        Ok(())
    }
}

// Note: Conversion functions moved to conversion.rs module
// The module provides enhanced conversion with:
// - Table to Object/Array conversion
// - Nested structure support
// - Cycle detection
// - Recursion depth limits
