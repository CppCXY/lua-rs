use luars::{Lua, LuaApi, LuaValue, SafeOption, Stdlib};
use wasm_bindgen::prelude::*;

mod conversion;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Lua 5.5 lua for WebAssembly.
#[wasm_bindgen]
pub struct LuaWasm {
    lua: Lua,
}

#[wasm_bindgen]
impl LuaWasm {
    /// Create a new lua with all standard libraries loaded.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<LuaWasm, JsValue> {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        Ok(LuaWasm { lua })
    }

    /// Execute Lua source code. Returns the first result as a string,
    /// or throws on error.
    #[wasm_bindgen]
    pub fn execute(&mut self, code: &str) -> Result<String, JsValue> {
        match self.lua.load(code).eval::<LuaValue>() {
            Ok(result) => Ok(format!("{}", result)),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(&msg.message()))
            }
        }
    }

    /// Execute Lua code and return **all** results as a JS Array.
    /// Each Lua value is converted to its JS equivalent (tables → objects/arrays, etc.).
    #[wasm_bindgen(js_name = executeMulti)]
    pub fn execute_multi(&mut self, code: &str) -> Result<JsValue, JsValue> {
        match self.lua.vm_mut().execute(code) {
            Ok(results) => Ok(self.lua_results_to_js_array(&results)),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    /// Execute Lua code and return the first result as a JS value
    /// (with full table conversion).
    #[wasm_bindgen(js_name = doString)]
    pub fn do_string(&mut self, code: &str) -> Result<JsValue, JsValue> {
        match self.lua.load(code).eval::<LuaValue>() {
            Ok(first) => Ok(conversion::lua_to_js(self.lua.vm_mut(), &first)),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    /// Evaluate a Lua **expression** (auto-prepends `return`).
    /// Returns the result as a JS value with full type conversion.
    #[wasm_bindgen]
    pub fn eval(&mut self, expr: &str) -> Result<JsValue, JsValue> {
        let code = format!("return {}", expr);
        match self.lua.load(&code).eval::<LuaValue>() {
            Ok(first) => Ok(conversion::lua_to_js(self.lua.vm_mut(), &first)),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    /// Compile Lua source without executing it. Returns `true` on success.
    /// Useful to syntax-check code before running it.
    #[wasm_bindgen]
    pub fn check(&mut self, code: &str) -> Result<bool, JsValue> {
        match self.lua.load(code).into_function() {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    // ── Globals ──────────────────────────────────────────────────────

    /// Set a global variable. JS values are automatically converted:
    /// - `null`/`undefined` → `nil`
    /// - `boolean` → `boolean`
    /// - `number` → `integer` or `float`
    /// - `string` → `string`
    /// - `Array` → sequence table `{[1]=…, [2]=…}`
    /// - `Object` → hash table `{key=…}`
    #[wasm_bindgen(js_name = setGlobal)]
    pub fn set_global(&mut self, name: &str, value: JsValue) -> Result<(), JsValue> {
        let lua_value = conversion::js_to_lua(self.lua.vm_mut(), &value)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.lua
            .set_global(name, lua_value)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        Ok(())
    }

    /// Get a global variable, returned as a JS value with full conversion.
    /// Returns `null` if the global does not exist or is `nil`.
    #[wasm_bindgen(js_name = getGlobal)]
    pub fn get_global(&mut self, name: &str) -> Result<JsValue, JsValue> {
        match self.lua.get_global::<LuaValue>(name) {
            Ok(Some(v)) => Ok(conversion::lua_to_js(self.lua.vm_mut(), &v)),
            Ok(None) => Ok(JsValue::NULL),
            Err(e) => Err(JsValue::from_str(&format!("{:?}", e))),
        }
    }

    // ── Register JS functions into Lua ───────────────────────────────

    /// Register a JavaScript function as a Lua global.
    ///
    /// The JS callback receives an array of arguments and should return a
    /// single value (or `undefined` / `null` for no return).
    ///
    /// ```js
    /// lua.registerFunction("greet", (args) => "Hello, " + args[0]);
    /// lua.execute('print(greet("world"))'); // Hello, world
    /// ```
    #[wasm_bindgen(js_name = registerFunction)]
    pub fn register_function(
        &mut self,
        name: &str,
        callback: js_sys::Function,
    ) -> Result<(), JsValue> {
        let closure = self
            .lua
            .vm_mut()
            .create_closure(move |state: &mut luars::LuaState| {
                // Collect Lua arguments → JS Array
                let args = js_sys::Array::new();
                for i in 1..=state.arg_count() {
                    if let Some(v) = state.get_arg(i) {
                        args.push(&conversion::lua_to_js_basic(&v));
                    }
                }
                // Call JS callback
                let result = callback
                    .call1(&JsValue::NULL, &args)
                    .map_err(|e| state.error(format!("JS callback error: {:?}", e)))?;
                // Convert JS result → Lua
                if result.is_undefined() || result.is_null() {
                    Ok(0)
                } else {
                    let lua_val =
                        conversion::js_to_lua_basic(state, &result).map_err(|e| state.error(e))?;
                    state.push_value(lua_val)?;
                    Ok(1)
                }
            })
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.lua
            .vm_mut()
            .set_global(name, closure)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        Ok(())
    }

    // ── Calling Lua functions from JS ────────────────────────────────

    /// Call a global Lua function by name with JS arguments.
    /// Returns all results as a JS Array.
    ///
    /// ```js
    /// lua.execute('function add(a,b) return a+b end');
    /// const r = lua.callGlobal("add", [1, 2]); // [3]
    /// ```
    #[wasm_bindgen(js_name = callGlobal)]
    pub fn call_global(
        &mut self,
        name: &str,
        args: Option<js_sys::Array>,
    ) -> Result<JsValue, JsValue> {
        let lua_args = self.js_array_to_lua_args(args)?;
        match self.lua.vm_mut().call_global_raw(name, lua_args) {
            Ok(results) => Ok(self.lua_results_to_js_array(&results)),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    /// Call a Lua value (function / callable table) with JS arguments.
    /// The first argument is a Lua function previously obtained (e.g. from `getGlobal`).
    /// Returns all results as a JS Array.
    #[wasm_bindgen(js_name = callFunction)]
    pub fn call_function(
        &mut self,
        func_name: &str,
        args: Option<js_sys::Array>,
    ) -> Result<JsValue, JsValue> {
        // Lookup the function by name, then call it
        let func = self
            .lua
            .vm_mut()
            .get_global(func_name)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?
            .ok_or_else(|| JsValue::from_str(&format!("global '{}' not found", func_name)))?;
        let lua_args = self.js_array_to_lua_args(args)?;
        match self.lua.vm_mut().call_raw(func, lua_args) {
            Ok(results) => Ok(self.lua_results_to_js_array(&results)),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────

    fn js_array_to_lua_args(
        &mut self,
        args: Option<js_sys::Array>,
    ) -> Result<Vec<LuaValue>, JsValue> {
        match args {
            Some(arr) => {
                let mut lua_args = Vec::with_capacity(arr.length() as usize);
                for i in 0..arr.length() {
                    let v = conversion::js_to_lua(self.lua.vm_mut(), &arr.get(i))
                        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
                    lua_args.push(v);
                }
                Ok(lua_args)
            }
            None => Ok(Vec::new()),
        }
    }

    fn lua_results_to_js_array(&mut self, results: &[LuaValue]) -> JsValue {
        let arr = js_sys::Array::new();
        for v in results {
            arr.push(&conversion::lua_to_js(self.lua.vm_mut(), v));
        }
        arr.into()
    }

    // ── Output capture ──────────────────────────────────────────────

    /// Redirect Lua `print()` to a JavaScript callback.
    ///
    /// The callback receives a single string (the concatenated printed line).
    ///
    /// ```js
    /// lua.onPrint((line) => document.getElementById("output").textContent += line + "\n");
    /// lua.execute('print("hello", "world")'); // callback receives "hello\tworld"
    /// ```
    #[wasm_bindgen(js_name = onPrint)]
    pub fn on_print(&mut self, callback: js_sys::Function) -> Result<(), JsValue> {
        let closure = self
            .lua
            .vm_mut()
            .create_closure(move |state: &mut luars::LuaState| {
                let mut parts = Vec::new();
                for i in 1..=state.arg_count() {
                    if let Some(v) = state.get_arg(i) {
                        parts.push(format!("{}", v));
                    }
                }
                let line = parts.join("\t");
                let _ = callback.call1(&JsValue::NULL, &JsValue::from_str(&line));
                Ok(0)
            })
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.lua
            .vm_mut()
            .set_global("print", closure)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        Ok(())
    }

    /// Redirect Lua `io.write()` to a JavaScript callback.
    ///
    /// Unlike `onPrint`, the callback receives each write call's text
    /// without automatic newline or tab joining.
    #[wasm_bindgen(js_name = onWrite)]
    pub fn on_write(&mut self, callback: js_sys::Function) -> Result<(), JsValue> {
        // Override io.write via Lua code that calls our registered function
        let closure = self
            .lua
            .vm_mut()
            .create_closure(move |state: &mut luars::LuaState| {
                let mut out = String::new();
                for i in 1..=state.arg_count() {
                    if let Some(v) = state.get_arg(i) {
                        out.push_str(&format!("{}", v));
                    }
                }
                let _ = callback.call1(&JsValue::NULL, &JsValue::from_str(&out));
                Ok(0)
            })
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.lua
            .vm_mut()
            .set_global("__wasm_io_write", closure)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        // Wire it into io.write
        self.lua.execute("io.write = __wasm_io_write").map_err(|e| {
            let msg = self.lua.get_error_message(e);
            JsValue::from_str(msg.message())
        })?;
        Ok(())
    }

    // ── Table creation ──────────────────────────────────────────────

    /// Create a Lua table from a JS Object or Array and set it as a global.
    ///
    /// ```js
    /// lua.createTable("config", { width: 800, height: 600, title: "Game" });
    /// lua.createTable("items", [10, 20, 30]);
    /// lua.execute('print(config.title)'); // "Game"
    /// ```
    #[wasm_bindgen(js_name = createTable)]
    pub fn create_table(&mut self, name: &str, data: JsValue) -> Result<(), JsValue> {
        let lua_val = conversion::js_to_lua(self.lua.vm_mut(), &data)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.lua
            .set_global(name, lua_val)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        Ok(())
    }

    /// Get a Lua table field by key path (dot-separated).
    /// E.g. `lua.getField("config.window.width")`.
    #[wasm_bindgen(js_name = getField)]
    pub fn get_field(&mut self, path: &str) -> Result<JsValue, JsValue> {
        let code = format!("return {}", path);
        match self.lua.load(&code).eval::<LuaValue>() {
            Ok(first) => Ok(conversion::lua_to_js(self.lua.vm_mut(), &first)),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    /// Set a Lua table field using a dot-separated path.
    /// E.g. `lua.setField("config.debug", true)`.
    #[wasm_bindgen(js_name = setField)]
    pub fn set_field(&mut self, path: &str, value: JsValue) -> Result<(), JsValue> {
        // We set a temporary global, then assign via Lua
        let lua_val = conversion::js_to_lua(self.lua.vm_mut(), &value)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.lua
            .set_global("__wasm_tmp", lua_val)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        let code = format!("{} = __wasm_tmp; __wasm_tmp = nil", path);
        self.lua.execute(&code).map_err(|e| {
            let msg = self.lua.get_error_message(e);
            JsValue::from_str(msg.message())
        })?;
        Ok(())
    }

    // ── GC & lua management ──────────────────────────────────────────

    /// Run a full garbage collection cycle.
    #[wasm_bindgen(js_name = collectGarbage)]
    pub fn collect_garbage(&mut self) -> Result<(), JsValue> {
        self.lua.collect_garbage().map_err(|e| {
            let msg = self.lua.get_error_message(e);
            JsValue::from_str(msg.message())
        })?;
        Ok(())
    }

    /// Get GC statistics as a string.
    #[wasm_bindgen(js_name = gcStats)]
    pub fn gc_stats(&mut self) -> String {
        self.lua.vm_mut().gc_stats()
    }

    /// Get the approximate memory usage in bytes.
    #[wasm_bindgen(js_name = memoryUsed)]
    pub fn memory_used(&mut self) -> Result<f64, JsValue> {
        match self.lua.load("return collectgarbage('count')").eval::<f64>() {
            Ok(kb) => Ok(kb * 1024.0),
            Err(e) => {
                let msg = self.lua.get_error_message(e);
                Err(JsValue::from_str(msg.message()))
            }
        }
    }

    /// Reset the lua: creates a fresh lua with all standard libraries.
    /// All previous state (globals, functions, tables) is discarded.
    #[wasm_bindgen]
    pub fn reset(&mut self) -> Result<(), JsValue> {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        self.lua = lua;
        Ok(())
    }

    // ── Compile / load ──────────────────────────────────────────────

    /// Compile Lua source into a callable function without executing it.
    /// The returned function is stored as a global with the given name.
    ///
    /// ```js
    /// lua.load("myFunc", "return 42");
    /// const r = lua.callGlobal("myFunc", []);
    /// ```
    #[wasm_bindgen]
    pub fn load(&mut self, name: &str, code: &str) -> Result<(), JsValue> {
        let func = self.lua.load(code).into_function().map_err(|e| {
            let msg = self.lua.get_error_message(e);
            JsValue::from_str(msg.message())
        })?;
        self.lua
            .set_global(name, func)
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        Ok(())
    }
}
