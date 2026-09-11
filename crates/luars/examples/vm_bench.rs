//! Developer benchmark entry: run a single Lua script through the VM.
//!
//! Usage:
//!   cargo run -p luars --release --example vm_bench -- script.lua

use std::{env, fs, process};

use luars::{Lua, LuaApi, SafeOption, Stdlib};

fn main() {
    let args: Vec<String> = env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: vm_bench <script.lua>");
        process::exit(2);
    };
    let code = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", path);
        process::exit(2);
    });
    let mut lua = Lua::new(SafeOption::default());
    if let Err(e) = lua.open_stdlib(Stdlib::All) {
        eprintln!("failed to open stdlib: {e:?}");
        process::exit(2);
    }
    if let Err(e) = lua.load(&code).set_name(format!("@{path}")).exec() {
        eprintln!("{}", lua.get_error_message(e));
        process::exit(1);
    }
}
