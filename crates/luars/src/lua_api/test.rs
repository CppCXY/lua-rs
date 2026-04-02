#[cfg(test)]
mod tests {
    use std::cell::Cell;

    #[cfg(feature = "sandbox")]
    use crate::SandboxConfig;
    #[cfg(feature = "serde")]
    use crate::lua_api::Value;
    use crate::{
        LuaUserData, SafeOption, Stdlib,
        lua_api::{Function, Lua, Table},
        lua_methods,
    };
    #[cfg(feature = "serde")]
    use serde::{Deserialize, Serialize};

    #[cfg(feature = "serde")]
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct ApiConfig {
        host: String,
        port: u16,
        tags: Vec<String>,
    }

    #[derive(LuaUserData)]
    struct ApiCounter {
        pub count: i64,
    }

    #[lua_methods]
    impl ApiCounter {
        pub fn inc(&mut self, delta: i64) {
            self.count += delta;
        }

        pub fn get(&self) -> i64 {
            self.count
        }
    }

    #[test]
    fn eval_and_typed_globals_work() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        lua.set_global("name", "Lua").unwrap();

        let result: String = lua.eval("return 'hello ' .. name").unwrap();
        assert_eq!(result, "hello Lua");
    }

    #[test]
    fn register_and_call_typed_function() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        lua.register_function("sum", |a: i64, b: i64| a + b)
            .unwrap();

        let result: i64 = lua.eval("return sum(20, 22)").unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn call_global_for_lua_defined_function() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();
        lua.load("function mul(a, b) return a * b end")
            .exec()
            .unwrap();

        let result: i64 = lua.call_global1("mul", (6, 7)).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn safe_table_round_trip() {
        let mut lua = Lua::new(SafeOption::default());
        let table = lua.create_table_with_capacity(0, 2).unwrap();

        lua.table_set(&table, "host", "localhost").unwrap();
        lua.table_set(&table, "port", 8080_i64).unwrap();
        lua.set_global_table("config", &table).unwrap();

        let config = lua.get_table("config").unwrap().unwrap();
        let host: String = config.get("host").unwrap();
        let port: i64 = config.get("port").unwrap();

        assert_eq!(host, "localhost");
        assert_eq!(port, 8080);
    }

    #[test]
    fn globals_and_generic_table_api_feel_like_mlua() {
        let mut lua = Lua::new(SafeOption::default());
        let globals = lua.globals();

        globals.set("host", "localhost").unwrap();
        globals.set("port", 8080_i64).unwrap();

        assert!(globals.contains_key("host").unwrap());
        assert_eq!(globals.get::<String>("host").unwrap(), "localhost");
        assert_eq!(globals.raw_get::<i64>("port").unwrap(), 8080);
    }

    #[test]
    fn create_table_from_and_sequence_from_work() {
        let mut lua = Lua::new(SafeOption::default());

        let config = lua
            .create_table_from([("host", "localhost"), ("mode", "dev")])
            .unwrap();
        let seq = lua.create_sequence_from([10_i64, 20_i64, 30_i64]).unwrap();

        assert_eq!(config.get::<String>("host").unwrap(), "localhost");
        assert_eq!(config.pairs::<String, String>().unwrap().len(), 2);
        assert_eq!(seq.sequence_values::<i64>().unwrap(), vec![10, 20, 30]);
    }

    #[test]
    fn create_function_and_convert_helpers_work() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let double = lua.create_function(|x: i64| x * 2).unwrap();
        lua.globals().set("double", double.clone()).unwrap();

        let packed = lua.pack("42").unwrap();
        let unpacked: String = lua.unpack(packed).unwrap();
        let converted: i64 = lua.convert(123_i64).unwrap();
        let result: i64 = lua.eval("return double(21)").unwrap();

        assert_eq!(unpacked, "42");
        assert_eq!(converted, 123);
        assert_eq!(result, 42);
    }

    #[test]
    fn table_objectlike_helpers_work() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let obj: Table = lua
            .load(
                r#"
                return {
                    nested = { answer = 42 },
                    add = function(a, b) return a + b end,
                    scale = function(self, x) return self.factor * x end,
                    factor = 3,
                }
                "#,
            )
            .eval()
            .unwrap();

        assert_eq!(obj.get_path::<i64>(&["nested", "answer"]).unwrap(), 42);
        assert_eq!(obj.call_function::<_, i64>("add", (20, 22)).unwrap(), 42);
        assert_eq!(obj.call_method1::<_, i64>("scale", 14_i64).unwrap(), 42);
    }

    #[test]
    fn safe_value_handle_supports_string_and_downcasts() {
        let mut lua = Lua::new(SafeOption::default());

        let string_value = lua.pack("hello").unwrap();
        let table = lua.create_table_from([("answer", 42_i64)]).unwrap();
        let table_value = lua.pack(table).unwrap();
        let userdata = lua.create_userdata(ApiCounter { count: 1 }).unwrap();
        let userdata_value = lua.pack(userdata.clone()).unwrap();

        assert_eq!(string_value.type_name(), "string");
        assert_eq!(string_value.as_string().unwrap(), "hello");
        assert_eq!(string_value.to_string_lossy(), "hello");
        assert_eq!(
            string_value.as_string_handle().unwrap().as_str(),
            Some("hello")
        );

        let table = table_value.as_table().unwrap();
        assert_eq!(table.get::<i64>("answer").unwrap(), 42);

        let counter = userdata_value.as_userdata::<ApiCounter>().unwrap();
        assert_eq!(counter.get().unwrap().count, 1);

        let converted: String = string_value.get().unwrap();
        assert_eq!(converted, "hello");
    }

    #[test]
    fn high_level_userdata_api_works() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let type_table = lua.register_type::<ApiCounter>("Counter").unwrap();
        assert!(type_table.raw_len().is_ok());

        let counter = lua.create_userdata(ApiCounter { count: 1 }).unwrap();
        lua.globals().set("counter", counter.clone()).unwrap();
        lua.load("counter:inc(41)").exec().unwrap();

        assert_eq!(counter.get().unwrap().count, 42);
        assert_eq!(lua.load("return counter:get()").eval::<i64>().unwrap(), 42);
    }

    #[test]
    fn borrowed_userdata_api_works() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let mut counter = ApiCounter { count: 2 };
        let borrowed = unsafe { lua.create_userdata_ref(&mut counter).unwrap() };
        lua.globals().set("borrowed", borrowed.clone()).unwrap();
        lua.load("borrowed:inc(40)").exec().unwrap();

        assert_eq!(counter.count, 42);
        assert_eq!(borrowed.get().unwrap().count, 42);
    }

    #[test]
    fn scope_supports_non_static_functions() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let base = 40_i64;
        lua.scope(|scope| {
            let add_base = scope.create_function_with(&base, |base: &i64, x: i64| x + *base)?;
            scope.globals().set("add_base", &add_base)?;

            let result: i64 = scope.load("return add_base(2)").eval()?;
            assert_eq!(result, 42);
            Ok(())
        })
        .unwrap();

        assert!(lua.load("return add_base(1)").eval::<i64>().is_err());
    }

    #[test]
    fn scope_supports_borrowed_userdata() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let mut counter = ApiCounter { count: 1 };
        lua.scope(|scope| {
            let mut borrowed = scope.create_userdata_ref(&mut counter)?;
            scope.globals().set("borrowed", &borrowed)?;

            let count: i64 = scope.load("return borrowed.count").eval()?;
            assert_eq!(count, 1);
            let called: i64 = scope
                .load("borrowed:inc(41); return borrowed:get()")
                .eval()?;
            assert_eq!(called, 42);
            let reassigned: i64 = scope
                .load("borrowed.count = borrowed.count + 1; return borrowed.count")
                .eval()?;
            assert_eq!(reassigned, 43);

            borrowed.get_mut()?.inc(41);
            assert_eq!(borrowed.get()?.count, 84);
            Ok(())
        })
        .unwrap();

        assert_eq!(counter.count, 84);

        assert!(lua.load("return borrowed:get()").eval::<i64>().is_err());
        assert!(lua.load("borrowed.count = 1").exec().is_err());
    }

    #[test]
    fn scope_function_with_borrowed_state_works() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let total = Cell::new(0_i64);
        lua.scope(|scope| {
            let push = scope.create_function_with(&total, |total: &Cell<i64>, delta: i64| {
                total.set(total.get() + delta);
                total.get()
            })?;
            scope.globals().set("push_total", &push)?;

            let value: i64 = scope
                .load("return push_total(19) + push_total(23)")
                .eval()?;
            assert_eq!(value, 61);
            Ok(())
        })
        .unwrap();

        assert_eq!(total.get(), 42);
    }

    #[test]
    fn scope_function_mut_with_borrowed_state_works() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let mut total = 0_i64;
        lua.scope(|scope| {
            let push =
                scope.create_function_mut_with(&mut total, |total: &mut i64, delta: i64| {
                    *total += delta;
                    *total
                })?;
            scope.globals().set("push_total_mut", &push)?;

            let value: i64 = scope
                .load("return push_total_mut(19) + push_total_mut(23)")
                .eval()?;
            assert_eq!(value, 61);
            Ok(())
        })
        .unwrap();

        assert_eq!(total, 42);
    }

    #[test]
    fn scope_function_with_borrowed_reference_works() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let base = 40_i64;
        lua.scope(|scope| {
            let add_base = scope.create_function_with(&base, |base: &i64, x: i64| x + *base)?;
            scope.globals().set("add_base", &add_base)?;

            let result: i64 = scope.load("return add_base(2)").eval()?;
            assert_eq!(result, 42);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn chunk_builder_exec_eval_and_into_function_work() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        lua.load("answer = 41").set_name("init.lua").exec().unwrap();
        let answer: i64 = lua.load("return answer + 1").eval().unwrap();
        let add = lua
            .load("local a, b = ...; return a + b")
            .set_name("adder.lua")
            .into_function()
            .unwrap();

        assert_eq!(answer, 42);
        assert_eq!(add.call1::<_, i64>((20, 22)).unwrap(), 42);
    }

    #[tokio::test]
    async fn high_level_async_api_exec_and_call_work() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        lua.register_async_function("double_async", |x: i64| async move { Ok(x * 2) })
            .unwrap();
        lua.load(
            r#"
            function add_async(a, b)
                return double_async(a + b)
            end
            "#,
        )
        .exec()
        .unwrap();

        let chunk_value: i64 = lua
            .load("return double_async(21)")
            .eval_async()
            .await
            .unwrap();
        let global_value: i64 = lua
            .call_async_global1("add_async", (20_i64, 1_i64))
            .await
            .unwrap();
        let compiled: Function = lua
            .load("return function(x) return double_async(x) end")
            .eval()
            .unwrap();
        let function_value: i64 = lua.call_async1(&compiled, 21_i64).await.unwrap();

        assert_eq!(chunk_value, 42);
        assert_eq!(global_value, 42);
        assert_eq!(function_value, 42);
    }

    #[cfg(feature = "sandbox")]
    #[test]
    fn high_level_sandbox_api_supports_injected_globals_and_isolation() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();
        lua.register_function("greet", |name: String| format!("hello, {name}"))
            .unwrap();

        let mut config = SandboxConfig::default();
        lua.sandbox_capture_global(&mut config, "greet").unwrap();
        let value: String = lua
            .load_sandboxed(
                r#"
                sandbox_value = 41
                return greet("sandbox")
                "#,
                &config,
            )
            .eval()
            .unwrap();

        assert_eq!(value, "hello, sandbox");
        assert!(lua.get_global::<i64>("sandbox_value").unwrap().is_none());
    }

    #[test]
    fn table_and_function_convert_from_lua() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let table: Table = lua
            .load("return { host = 'localhost', port = 8080 }")
            .eval()
            .unwrap();
        let function: Function = lua
            .load("return function(x) return x * 2 end")
            .eval()
            .unwrap();

        assert_eq!(table.get::<String>("host").unwrap(), "localhost");
        assert_eq!(table.get::<i64>("port").unwrap(), 8080);
        assert_eq!(function.call1::<_, i64>(21).unwrap(), 42);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn table_serde_json_round_trip_works() {
        let mut lua = Lua::new(SafeOption::default());
        let table = lua
            .create_table_from([("host", "localhost"), ("port", "8080")])
            .unwrap();
        table
            .set(
                "nested",
                lua.create_sequence_from([1_i64, 2_i64, 3_i64]).unwrap(),
            )
            .unwrap();

        let json = table.to_json_value().unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "host": "localhost",
                "port": "8080",
                "nested": [1, 2, 3]
            })
        );

        let encoded = serde_json::to_value(&table).unwrap();
        assert_eq!(encoded, json);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn table_from_json_and_to_serde_work() {
        let mut lua = Lua::new(SafeOption::default());
        let table = Table::from_json_value(
            &mut lua,
            &serde_json::json!({
                "host": "127.0.0.1",
                "port": 8080,
                "tags": ["dev", "edge"]
            }),
        )
        .unwrap();

        let config: ApiConfig = table.to_serde().unwrap();
        assert_eq!(
            config,
            ApiConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                tags: vec!["dev".to_string(), "edge".to_string()],
            }
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn table_from_serde_works() {
        let mut lua = Lua::new(SafeOption::default());
        let input = ApiConfig {
            host: "localhost".to_string(),
            port: 3000,
            tags: vec!["api".to_string(), "beta".to_string()],
        };

        let table = Table::from_serde(&mut lua, &input).unwrap();
        assert_eq!(table.get::<String>("host").unwrap(), "localhost");
        assert_eq!(table.get::<i64>("port").unwrap(), 3000);
        assert_eq!(
            table
                .get::<Table>("tags")
                .unwrap()
                .sequence_values::<String>()
                .unwrap(),
            vec!["api".to_string(), "beta".to_string()]
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn value_serde_scalar_round_trip_works() {
        let mut lua = Lua::new(SafeOption::default());
        let value = lua.pack(42_i64).unwrap();

        assert_eq!(value.to_json_value().unwrap(), serde_json::json!(42));
        assert_eq!(serde_json::to_value(&value).unwrap(), serde_json::json!(42));

        let decoded: i64 = value.to_serde().unwrap();
        assert_eq!(decoded, 42);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn value_from_json_and_from_serde_work() {
        let mut lua = Lua::new(SafeOption::default());

        let from_json = Value::from_json_value(
            &mut lua,
            &serde_json::json!({
                "host": "127.0.0.1",
                "port": 8081,
                "tags": ["prod", "edge"]
            }),
        )
        .unwrap();
        let config: ApiConfig = from_json.to_serde().unwrap();
        assert_eq!(
            config,
            ApiConfig {
                host: "127.0.0.1".to_string(),
                port: 8081,
                tags: vec!["prod".to_string(), "edge".to_string()],
            }
        );

        let from_serde = Value::from_serde(
            &mut lua,
            &ApiConfig {
                host: "localhost".to_string(),
                port: 3001,
                tags: vec!["api".to_string()],
            },
        )
        .unwrap();

        let table = from_serde.as_table().unwrap();
        assert_eq!(table.get::<String>("host").unwrap(), "localhost");
        assert_eq!(table.get::<i64>("port").unwrap(), 3001);
        assert_eq!(
            table
                .get::<Table>("tags")
                .unwrap()
                .sequence_values::<String>()
                .unwrap(),
            vec!["api".to_string()]
        );
    }
}
