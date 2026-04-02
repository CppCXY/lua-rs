#[cfg(test)]
mod tests {
    use crate::{
        SafeOption, Stdlib,
        lua_api::{Function, Lua, Table},
    };

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
        lua.execute("function mul(a, b) return a * b end").unwrap();

        let result: i64 = lua.call_global1("mul", (6, 7)).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn safe_table_round_trip() {
        let mut lua = Lua::new(SafeOption::default());
        let table = lua.create_table(0, 2).unwrap();

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
    fn table_and_function_convert_from_lua() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let table: Table = lua
            .eval("return { host = 'localhost', port = 8080 }")
            .unwrap();
        let function: Function = lua.eval("return function(x) return x * 2 end").unwrap();

        assert_eq!(table.get::<String>("host").unwrap(), "localhost");
        assert_eq!(table.get::<i64>("port").unwrap(), 8080);
        assert_eq!(function.call1::<_, i64>(21).unwrap(), 42);
    }
}
