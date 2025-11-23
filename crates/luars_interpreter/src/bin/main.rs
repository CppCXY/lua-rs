use lua_rs::LuaVM;
use std::env;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::rc::Rc;

const VERSION: &str = "Lua-RS 5.4 (compatible)";
const COPYRIGHT: &str = "Copyright (C) 2025 lua-rs contributors";

fn print_usage() {
    eprintln!("usage: lua [options] [script [args]]");
    eprintln!("Available options are:");
    eprintln!("  -e stat   execute string 'stat'");
    eprintln!("  -i        enter interactive mode after executing 'script'");
    eprintln!("  -l mod    require library 'mod' into global 'mod'");
    eprintln!("  -v        show version information");
    eprintln!("  -E        ignore environment variables");
    eprintln!("  -W        turn warnings on");
    eprintln!("  --        stop handling options");
    eprintln!("  -         stop handling options and execute stdin");
}

fn print_version() {
    println!("{}", VERSION);
    println!("{}", COPYRIGHT);
}

struct Options {
    execute_strings: Vec<String>,
    interactive: bool,
    script_file: Option<String>,
    script_args: Vec<String>,
    require_modules: Vec<String>,
    show_version: bool,
    read_stdin: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            execute_strings: Vec::new(),
            interactive: false,
            script_file: None,
            script_args: Vec::new(),
            require_modules: Vec::new(),
            show_version: false,
            read_stdin: false,
        }
    }
}

fn parse_args() -> Result<Options, String> {
    let args: Vec<String> = env::args().collect();
    let mut opts = Options::default();
    let mut i = 1;
    let mut stop_options = false;

    while i < args.len() {
        let arg = &args[i];

        if !stop_options && arg.starts_with('-') {
            match arg.as_str() {
                "-e" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("'-e' needs argument".to_string());
                    }
                    opts.execute_strings.push(args[i].clone());
                }
                "-i" => {
                    opts.interactive = true;
                }
                "-l" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("'-l' needs argument".to_string());
                    }
                    opts.require_modules.push(args[i].clone());
                }
                "-v" => {
                    opts.show_version = true;
                }
                "-E" => {
                    // Ignore environment variables (not implemented)
                }
                "-W" => {
                    // Turn warnings on (not implemented)
                }
                "--" => {
                    stop_options = true;
                }
                "-" => {
                    opts.read_stdin = true;
                    stop_options = true;
                }
                _ => {
                    return Err(format!("unrecognized option '{}'", arg));
                }
            }
        } else {
            // First non-option argument is the script file
            opts.script_file = Some(arg.clone());
            i += 1;
            // Remaining arguments are script arguments
            while i < args.len() {
                opts.script_args.push(args[i].clone());
                i += 1;
            }
            break;
        }
        i += 1;
    }

    Ok(opts)
}

#[allow(dead_code)]
fn setup_arg_table(vm: &mut LuaVM, script_name: Option<&str>, args: &[String]) {
    // Create arg table: arg[-1] = interpreter, arg[0] = script, arg[1..] = arguments
    let code = format!(
        r#"
        arg = {{}}
        arg[-1] = "lua"
        arg[0] = {}
        "#,
        if let Some(name) = script_name {
            format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            "nil".to_string()
        }
    );

    let mut full_code = code;
    for (i, arg) in args.iter().enumerate() {
        full_code.push_str(&format!(
            "arg[{}] = \"{}\"\n",
            i + 1,
            arg.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }

    if let Ok(chunk) = vm.compile(&full_code) {
        let _ = vm.execute(Rc::new(chunk));
    }
}

fn require_module(vm: &mut LuaVM, module: &str) -> Result<(), String> {
    let code = format!("{} = require('{}')", module, module);
    match vm.compile(&code) {
        Ok(chunk) => {
            vm.execute(Rc::new(chunk)).map_err(|e| format!("{}", e))?;
            Ok(())
        }
        Err(e) => Err(format!("failed to load module '{}': {}", module, e)),
    }
}

fn execute_file(vm: &mut LuaVM, filename: &str) -> Result<(), String> {
    let code =
        fs::read_to_string(filename).map_err(|e| format!("cannot open {}: {}", filename, e))?;

    match vm.compile(&code) {
        Ok(chunk) => {
            vm.execute(Rc::new(chunk)).map_err(|e| format!("{}", e))?;
            Ok(())
        }
        Err(e) => Err(format!("{}: {}", filename, e)),
    }
}

fn execute_stdin(vm: &mut LuaVM) -> Result<(), String> {
    let mut code = String::new();
    io::stdin()
        .read_to_string(&mut code)
        .map_err(|e| format!("error reading stdin: {}", e))?;

    match vm.compile(&code) {
        Ok(chunk) => {
            vm.execute(Rc::new(chunk)).map_err(|e| format!("{}", e))?;
            Ok(())
        }
        Err(e) => Err(format!("stdin: {}", e)),
    }
}

fn run_repl(vm: &mut LuaVM) {
    println!("{}", VERSION);
    println!("{}", COPYRIGHT);
    println!("Type Ctrl+C or Ctrl+Z to exit\n");

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut incomplete = String::new();

    loop {
        // Print prompt
        if incomplete.is_empty() {
            print!("> ");
        } else {
            print!(">> ");
        }
        io::stdout().flush().unwrap();

        // Read line
        let line = match lines.next() {
            Some(Ok(line)) => line,
            Some(Err(_)) | None => break,
        };

        // Check for exit commands
        let trimmed = line.trim();
        if incomplete.is_empty() && (trimmed == "exit" || trimmed == "quit") {
            break;
        }

        // Accumulate line
        if !incomplete.is_empty() {
            incomplete.push('\n');
        }
        incomplete.push_str(&line);

        // Try to execute as expression first (for immediate values)
        let expr_code = format!("return {}", incomplete);
        let try_expr = vm.compile(&expr_code);

        let code_to_run = if try_expr.is_ok() {
            expr_code
        } else {
            incomplete.clone()
        };

        // Try to compile and execute
        match vm.compile(&code_to_run) {
            Ok(chunk) => {
                match vm.execute(Rc::new(chunk)) {
                    Ok(result) => {
                        // Print non-nil results
                        if !result.is_nil() {
                            // Use Debug format for display
                            println!("{:?}", result);
                        }
                        incomplete.clear();
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        incomplete.clear();
                    }
                }
            }
            Err(e) => {
                // Check if error is due to incomplete input
                let error_msg = e.to_string();
                if error_msg.contains("<eof>") || error_msg.contains("expected") {
                    // Might be incomplete, keep accumulating
                    continue;
                } else {
                    eprintln!("{}", e);
                    incomplete.clear();
                }
            }
        }
    }
}

fn main() {
    let opts = match parse_args() {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("lua: {}", e);
            print_usage();
            std::process::exit(1);
        }
    };

    // Show version if requested
    if opts.show_version {
        print_version();
        if opts.execute_strings.is_empty() && opts.script_file.is_none() && !opts.read_stdin {
            return;
        }
    }

    // Create VM
    let mut vm = LuaVM::new();
    vm.open_libs();

    // Setup arg table
    // FIXME: Disabled due to compiler bug with negative table indices
    // setup_arg_table(&mut vm, opts.script_file.as_deref(), &opts.script_args);

    // Require modules
    for module in &opts.require_modules {
        if let Err(e) = require_module(&mut vm, module) {
            eprintln!("lua: {}", e);
            std::process::exit(1);
        }
    }

    // Execute all -e strings in order (they share the same VM state)
    for code in &opts.execute_strings {
        match vm.compile(code) {
            Ok(chunk) => {
                if let Err(e) = vm.execute(Rc::new(chunk)) {
                    eprintln!("lua: {}", e);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("lua: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Execute script file or stdin
    if let Some(filename) = &opts.script_file {
        if let Err(e) = execute_file(&mut vm, filename) {
            eprintln!("lua: {}", e);
            std::process::exit(1);
        }
    } else if opts.read_stdin {
        if let Err(e) = execute_stdin(&mut vm) {
            eprintln!("lua: {}", e);
            std::process::exit(1);
        }
    }

    // Enter interactive mode if requested or if no script was provided
    if opts.interactive
        || (opts.execute_strings.is_empty() && opts.script_file.is_none() && !opts.read_stdin)
    {
        run_repl(&mut vm);
    }
}
