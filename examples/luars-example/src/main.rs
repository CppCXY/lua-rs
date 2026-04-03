use luars::{Lua, LuaResult, LuaUserData, SafeOption, Stdlib, lua_methods};

#[derive(LuaUserData)]
struct Counter {
    pub count: i64,
}

#[lua_methods]
impl Counter {
    pub fn new(count: i64) -> Self {
        Self { count }
    }

    pub fn inc(&mut self, delta: i64) {
        self.count += delta;
    }

    pub fn get(&self) -> i64 {
        self.count
    }
}

fn main() -> LuaResult<()> {
    let mut lua = Lua::new(SafeOption::default());
    lua.load_stdlibs(Stdlib::All)?;

    lua.register_function("slugify", |name: String| {
        name.trim().to_lowercase().replace(' ', "-")
    })?;
    lua.register_type::<Counter>("Counter")?;

    let config = lua.create_table_from([("host", "localhost"), ("mode", "demo")])?;
    lua.globals().set("config", config)?;

    let (slug, host, count): (String, String, i64) = lua.scope(|scope| {
        let prefix = String::from("user:");
        let format_name = scope
            .create_function_with(&prefix, |prefix: &String, value: String| {
                format!("{prefix}{value}")
            })?;
        scope.globals().set("format_name", &format_name)?;

        scope
            .load(
                r#"
                local counter = Counter.new(1)
                counter:inc(41)
                return format_name(slugify("Hello Lua")), config.host, counter:get()
                "#,
            )
            .eval_multi()
    })?;

    println!("slug: {slug}");
    println!("host: {host}");
    println!("count: {count}");
    Ok(())
}
