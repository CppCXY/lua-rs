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

    /// Register a JavaScript callback that can be called from Lua.
    /// The JS function receives Lua arguments converted to JsValues and
    /// its return value is converted back to a Lua value.
    ///
    /// Supported types: nil, boolean, number, string.
    /// Tables and other complex types are passed as `null` / `nil`.
    #[wasm_bindgen(js_name = registerFunction)]
    pub fn register_function(
        &mut self,
        name: String,
        callback: js_sys::Function,
    ) -> Result<(), JsValue> {
        // Create an RClosure capturing the JS callback
        let closure_value = self
            .vm
            .create_closure(move |state: &mut luars::lua_vm::LuaState| {
                let nargs = state.arg_count();

                // Convert Lua arguments to a JS array
                let js_args = js_sys::Array::new_with_length(nargs as u32);
                for i in 0..nargs {
                    let lua_val = state.get_arg(i + 1).unwrap_or(luars::LuaValue::nil());
                    js_args.set(i as u32, lua_to_js_basic(&lua_val));
                }

                // Call the JS callback: callback.apply(null, args)
                let result = callback
                    .apply(&JsValue::NULL, &js_args)
                    .map_err(|e| state.error(format!("JS callback error: {:?}", e)))?;

                // Convert JS result back to Lua and push
                let lua_result = js_to_lua_basic(state, &result)
                    .map_err(|msg| state.error(format!("JS result conversion error: {}", msg)))?;
                state.push_value(lua_result)?;
                Ok(1)
            })
            .map_err(|e| JsValue::from_str(&format!("Failed to create closure: {:?}", e)))?;

        self.vm
            .set_global(&name, closure_value)
            .map_err(|e| JsValue::from_str(&format!("Failed to set global: {:?}", e)))?;

        Ok(())
    }
}

/// Simple Lua → JS conversion for basic types (no VM access needed).
fn lua_to_js_basic(value: &luars::LuaValue) -> JsValue {
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

/// Simple JS → Lua conversion for basic types.
fn js_to_lua_basic(
    state: &mut luars::lua_vm::LuaState,
    value: &JsValue,
) -> Result<luars::LuaValue, String> {
    if value.is_null() || value.is_undefined() {
        Ok(luars::LuaValue::nil())
    } else if let Some(b) = value.as_bool() {
        Ok(luars::LuaValue::boolean(b))
    } else if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            Ok(luars::LuaValue::integer(n as i64))
        } else {
            Ok(luars::LuaValue::number(n))
        }
    } else if let Some(s) = value.as_string() {
        state
            .create_string(&s)
            .map_err(|e| format!("create_string failed: {:?}", e))
    } else {
        Ok(luars::LuaValue::nil())
    }
}
