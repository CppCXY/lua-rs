#![doc = include_str!("../README.md")]

mod builder;
mod function;
mod lua;
mod table;
mod util;

pub use builder::TableBuilder;
pub use function::Function;
pub use lua::Lua;
pub use table::Table;

pub use luars::LuaUserData;
pub use luars::lua_methods;
pub use luars::lua_vm::SafeOption;
pub use luars::{
    FromLua, FromLuaMulti, IntoLua, LuaEnum, LuaMethodProvider, LuaRegistrable, LuaResult,
    LuaStaticMethodProvider, Stdlib, UdValue, UserDataBuilder, UserDataRef, UserDataTrait,
};

#[cfg(feature = "sandbox")]
pub use luars::SandboxConfig;

#[cfg(test)]
mod tests {
    use super::*;

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
    fn safe_table_builder_round_trip() {
        let mut lua = Lua::new(SafeOption::default());
        let table = lua
            .build_table(
                TableBuilder::new()
                    .set("host", "localhost")
                    .set("port", 8080_i64)
                    .push("alpha")
                    .push("beta"),
            )
            .unwrap();

        assert_eq!(table.get::<String>("host").unwrap(), "localhost");
        assert_eq!(table.get::<i64>("port").unwrap(), 8080);
        assert_eq!(lua.table_geti::<String>(&table, 1).unwrap(), "alpha");
        assert_eq!(lua.table_geti::<String>(&table, 2).unwrap(), "beta");
    }

    #[test]
    fn table_and_function_convert_into_lua() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let table = lua
            .build_table(TableBuilder::new().set("name", "Lua"))
            .unwrap();
        lua.set_global("config", &table).unwrap();

        let function: Function = lua.eval("return function(a, b) return a + b end").unwrap();
        lua.set_global("adder", &function).unwrap();

        let name: String = lua.eval("return config.name").unwrap();
        let total: i64 = lua.eval("return adder(20, 22)").unwrap();

        assert_eq!(name, "Lua");
        assert_eq!(total, 42);
    }

    #[test]
    fn table_and_function_convert_from_lua() {
        let mut lua = Lua::new(SafeOption::default());
        lua.open_stdlib(Stdlib::All).unwrap();

        let table: Table = lua.eval("return { host = 'localhost', port = 8080 }").unwrap();
        let function: Function = lua.eval("return function(x) return x * 2 end").unwrap();

        assert_eq!(table.get::<String>("host").unwrap(), "localhost");
        assert_eq!(table.get::<i64>("port").unwrap(), 8080);
        assert_eq!(function.call1::<_, i64>(21).unwrap(), 42);
    }

    #[test]
    fn table_traversal_helpers_work() {
        let mut lua = Lua::new(SafeOption::default());
        let pairs_table = lua
            .build_table(
                TableBuilder::new()
                    .set("alpha", 1_i64)
                    .set("beta", 2_i64),
            )
            .unwrap();
        let array_table = lua
            .build_table(TableBuilder::new().push("a").push("b").push("c"))
            .unwrap();

        let mut pairs = lua.table_pairs::<String, i64>(&pairs_table).unwrap();
        pairs.sort_by(|left, right| left.0.cmp(&right.0));

        let array = lua.table_array::<String>(&array_table).unwrap();
        let raw_pairs = pairs_table.pairs_raw().unwrap();

        assert_eq!(pairs, vec![("alpha".to_owned(), 1), ("beta".to_owned(), 2)]);
        assert_eq!(array, vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        assert_eq!(raw_pairs.len(), 2);
    }
}
