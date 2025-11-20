use lua_rs::LuaVM;
fn main() {
    let mut vm = LuaVM::new();
    vm.open_libs();
    let result = vm.execute_string(r#"
        return type(42)
    "#);
    match result {
        Ok(v) => println!("Success: {:?}", v),
        Err(e) => println!("Error: {}", e),
    }
}
