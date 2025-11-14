use lua_rs::{Compiler, LuaValue, VM};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::rc::Rc;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        // File mode
        let filename = &args[1];
        match fs::read_to_string(filename) {
            Ok(code) => {
                let mut vm = VM::new();
                match Compiler::compile(&code) {
                    Ok(chunk) => match vm.execute(Rc::new(chunk)) {
                        Ok(result) => {
                            if !result.is_nil() {
                                println!("Result: {:?}", result);
                            }
                        }
                        Err(e) => {
                            eprintln!("Runtime error: {}", e);
                            std::process::exit(1);
                        }
                    },
                    Err(e) => {
                        eprintln!("Compile error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to read file '{}': {}", filename, e);
                std::process::exit(1);
            }
        }
        return;
    }

    // REPL mode
    println!("Lua VM - Interactive REPL");
    println!("Type Lua code and press Enter. Type 'exit' to quit.\n");

    let mut vm = VM::new();

    // Example: Set some global values
    let version_str = vm.create_string("0.1.0".to_string());
    vm.set_global("version", LuaValue::String(version_str));

    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        let input = input.trim();
        if input == "exit" || input == "quit" {
            break;
        }

        if input.is_empty() {
            continue;
        }

        // Try to compile and execute
        match Compiler::compile(input) {
            Ok(chunk) => match vm.execute(Rc::new(chunk)) {
                Ok(result) => {
                    if !result.is_nil() {
                        println!("=> {:?}", result);
                    }
                }
                Err(e) => {
                    eprintln!("Runtime error: {}", e);
                }
            },
            Err(e) => {
                eprintln!("Compile error: {}", e);
            }
        }
    }

    println!("Goodbye!");
}

#[cfg(test)]
mod tests {
    use lua_rs::execute;

    #[test]
    fn test_arithmetic() {
        let code = r#"
            local a = 10
            local b = 20
            local c = a + b
            return c
        "#;

        let result = execute(code);
        assert!(result.is_ok());
        if let Ok(val) = result {
            assert_eq!(val.as_number(), Some(30.0));
        }
    }

    #[test]
    fn test_conditionals() {
        let code = r#"
            local x = 5
            if x > 3 then
                return true
            end
            return false
        "#;

        let result = execute(code);
        assert!(result.is_ok());
    }

    #[test]
    fn test_loops() {
        let code = r#"
            local sum = 0
            local i = 1
            while i <= 10 do
                sum = sum + i
                i = i + 1
            end
            return sum
        "#;

        let result = execute(code);
        assert!(result.is_ok());
        if let Ok(val) = result {
            assert_eq!(val.as_number(), Some(55.0));
        }
    }

    #[test]
    fn test_tables() {
        let code = r#"
            local t = {}
            t[1] = 42
            return t[1]
        "#;

        let result = execute(code);
        assert!(result.is_ok());
    }

    #[test]
    fn test_strings() {
        let code = r#"
            local s = "hello"
            return s
        "#;

        let result = execute(code);
        assert!(result.is_ok());
    }
}
