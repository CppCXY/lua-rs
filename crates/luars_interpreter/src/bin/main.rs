use luars::LuaVM;
use luars::LuaValue;
use luars::lua_vm::SafeOption;
use luars::stdlib;
use std::env;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::rc::Rc;

const VERSION: &str = "Lua-RS 5.5 (compatible)";
const COPYRIGHT: &str = "Copyright (C) 2026 lua-rs CppCXY";

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

#[derive(Default)]
struct Options {
    execute_strings: Vec<String>,
    interactive: bool,
    script_file: Option<String>,
    script_args: Vec<String>,
    require_modules: Vec<String>,
    show_version: bool,
    read_stdin: bool,
    ignore_env: bool,
    warnings_on: bool,
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
                    opts.ignore_env = true;
                }
                "-W" => {
                    opts.warnings_on = true;
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

fn setup_arg_table(vm: &mut LuaVM, exe_path: &str, script_name: Option<&str>, args: &[String]) {
    // Create arg table: arg[negative] = interpreter opts, arg[0] = script, arg[1..] = script args
    let arg_table = vm.create_table(args.len(), 2).unwrap();

    // arg[0] = script name (or nil)
    if let Some(name) = script_name {
        let s = vm.create_string(name).unwrap();
        vm.raw_seti(&arg_table, 0, s);
    }

    // arg[-1] = interpreter executable path
    let exe = vm.create_string(exe_path).unwrap();
    vm.raw_seti(&arg_table, -1, exe);

    // arg[1], arg[2], ... = script arguments
    for (i, a) in args.iter().enumerate() {
        let s = vm.create_string(a).unwrap();
        vm.raw_seti(&arg_table, (i + 1) as i64, s);
    }

    let _ = vm.set_global("arg", arg_table);
}

fn require_module(vm: &mut LuaVM, module: &str) -> Result<(), String> {
    let code = format!("{} = require('{}')", module, module);
    match vm.compile(&code) {
        Ok(chunk) => {
            vm.execute_chunk(Rc::new(chunk))
                .map_err(|e| format!("{}", e))?;
            Ok(())
        }
        Err(e) => Err(format!("failed to load module '{}': {}", module, e)),
    }
}

fn execute_file(vm: &mut LuaVM, filename: &str) -> Result<(), String> {
    let code =
        fs::read_to_string(filename).map_err(|e| format!("cannot open {}: {}", filename, e))?;

    match vm.compile_with_name(&code, filename) {
        Ok(chunk) => {
            match vm.execute_chunk(Rc::new(chunk)) {
                Ok(_) => Ok(()),
                Err(e) => {
                    // Generate traceback for uncaught runtime errors
                    let error_msg = vm.get_error_message(e);
                    let traceback = vm.generate_traceback(&error_msg);
                    Err(traceback.to_string())
                }
            }
        }
        Err(e) => Err(format!("{}: {}: {}", filename, e, vm.get_error_message(e))),
    }
}

fn execute_stdin(vm: &mut LuaVM) -> Result<(), String> {
    let mut code = String::new();
    io::stdin()
        .read_to_string(&mut code)
        .map_err(|e| format!("error reading stdin: {}", e))?;

    match vm.compile(&code) {
        Ok(chunk) => {
            vm.execute_chunk(Rc::new(chunk))
                .map_err(|e| format!("{}", e))?;
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
                match vm.execute_chunk(Rc::new(chunk)) {
                    Ok(results) => {
                        // Print non-nil first result
                        if let Some(first) = results.into_iter().next()
                            && !first.is_nil()
                        {
                            // Use Debug format for display
                            println!("{:?}", first);
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

/// Resolve a LUA_PATH/LUA_CPATH env var value.
/// Replaces ";;" with the default path (like standard Lua).
fn resolve_env_path(env_value: &str, default: &str) -> String {
    if let Some(pos) = env_value.find(";;") {
        // Replace ";;" with ";default;"
        let prefix = &env_value[..pos];
        let suffix = &env_value[pos + 2..];
        let mut result = String::new();
        if !prefix.is_empty() {
            result.push_str(prefix);
            result.push(';');
        }
        result.push_str(default);
        if !suffix.is_empty() {
            result.push(';');
            result.push_str(suffix);
        }
        result
    } else {
        env_value.to_string()
    }
}

fn main() {
    // Install crash handler on Windows to capture crash address
    #[cfg(windows)]
    unsafe {
        #[repr(C)]
        struct ExceptionRecord {
            exception_code: u32,
            exception_flags: u32,
            exception_record: *mut ExceptionRecord,
            exception_address: *mut std::ffi::c_void,
            number_parameters: u32,
            exception_information: [usize; 15],
        }
        #[repr(C)]
        struct ContextRecord {
            _data: [u8; 1232], // CONTEXT is large, we don't need fields
        }
        #[repr(C)]
        struct ExceptionPointers {
            exception_record: *mut ExceptionRecord,
            context_record: *mut ContextRecord,
        }
        type VectoredHandler = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;
        unsafe extern "system" fn crash_handler(info: *mut ExceptionPointers) -> i32 {
            unsafe {
                let record = &*(*info).exception_record;
                if record.exception_code == 0xC0000005 {
                    // ACCESS_VIOLATION
                    let addr = record.exception_address;
                    let rw = if record.number_parameters >= 1 {
                        record.exception_information[0]
                    } else {
                        99
                    };
                    let target = if record.number_parameters >= 2 {
                        record.exception_information[1]
                    } else {
                        0
                    };
                    eprintln!(
                        "CRASH: ACCESS_VIOLATION at {:p}, type={} (0=read,1=write), target=0x{:x}",
                        addr, rw, target
                    );
                    // Print a basic backtrace using std::backtrace
                    let bt = std::backtrace::Backtrace::force_capture();
                    eprintln!("Backtrace:\n{}", bt);
                    std::process::exit(99);
                }
                0 // EXCEPTION_CONTINUE_SEARCH
            }
        }
        unsafe extern "system" {
            fn AddVectoredExceptionHandler(
                first: u32,
                handler: VectoredHandler,
            ) -> *mut std::ffi::c_void;
        }
        AddVectoredExceptionHandler(1, crash_handler);
    }

    // Spawn a thread with a larger stack to handle deep pcall/lua_execute recursion.
    // Each pcall calls lua_execute recursively, and lua_execute has a large stack frame.
    // With max_call_depth=256, we need ~16MB to avoid native stack overflow.
    let stack_size = 16 * 1024 * 1024; // 16 MB
    let builder = std::thread::Builder::new()
        .name("lua-main".into())
        .stack_size(stack_size);

    let handler = builder
        .spawn(lua_main)
        .expect("Failed to spawn lua-main thread");

    match handler.join() {
        Ok(code) => std::process::exit(code),
        Err(_) => {
            eprintln!("lua: internal error (thread panicked)");
            std::process::exit(1);
        }
    }
}

fn lua_main() -> i32 {
    let opts = match parse_args() {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("lua: {}", e);
            print_usage();
            return 1;
        }
    };

    // Show version if requested
    if opts.show_version {
        print_version();
        if opts.execute_strings.is_empty() && opts.script_file.is_none() && !opts.read_stdin {
            return 0;
        }
    }

    // Create VM
    let mut vm = LuaVM::new(SafeOption {
        max_stack_size: 1000000, // LUAI_MAXSTACK (Lua 5.5)
        // 问就是rust在debug版本递归限制太小了
        max_call_depth: if cfg!(debug_assertions) { 25 } else { 256 },
        base_call_depth: if cfg!(debug_assertions) { 25 } else { 256 },
        max_memory_limit: 1024 * 1024 * 1024, // 1 GB
    });
    vm.open_stdlib(stdlib::Stdlib::All).unwrap();
    if cfg!(debug_assertions) {
        let _ = vm.set_global("DEBUG", LuaValue::boolean(true));
    }

    // Handle -E: apply environment variables to package.path/cpath BEFORE -E blocks them
    if !opts.ignore_env {
        // Override package.path from LUA_PATH_5_5 or LUA_PATH
        if let Some(env_path) = env::var("LUA_PATH_5_5")
            .ok()
            .or_else(|| env::var("LUA_PATH").ok())
        {
            let default_path = "./?.lua;./?/init.lua";
            let resolved = resolve_env_path(&env_path, default_path);
            let code = format!("package.path = '{}'", resolved.replace('\\', "\\\\").replace('\'', "\\'"));
            if let Ok(chunk) = vm.compile(&code) {
                let _ = vm.execute_chunk(Rc::new(chunk));
            }
        }
        // Override package.cpath from LUA_CPATH_5_5 or LUA_CPATH
        if let Some(env_cpath) = env::var("LUA_CPATH_5_5")
            .ok()
            .or_else(|| env::var("LUA_CPATH").ok())
        {
            let default_cpath = "./?.so;./?.dll;./?.dylib";
            let resolved = resolve_env_path(&env_cpath, default_cpath);
            let code = format!("package.cpath = '{}'", resolved.replace('\\', "\\\\").replace('\'', "\\'"));
            if let Ok(chunk) = vm.compile(&code) {
                let _ = vm.execute_chunk(Rc::new(chunk));
            }
        }
    }

    // Handle LUA_INIT (unless -E)
    if !opts.ignore_env {
        if let Some(init) = env::var("LUA_INIT_5_5")
            .ok()
            .or_else(|| env::var("LUA_INIT").ok())
        {
            if let Some(filename) = init.strip_prefix('@') {
                // Execute file
                if let Err(e) = execute_file(&mut vm, filename) {
                    eprintln!("lua: {}", e);
                    return 1;
                }
            } else {
                // Execute string
                match vm.compile(&init) {
                    Ok(chunk) => {
                        if let Err(e) = vm.execute_chunk(Rc::new(chunk)) {
                            let error_msg = vm.get_error_message(e);
                            eprintln!("lua: {}", error_msg);
                            return 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("lua: {}", e);
                        return 1;
                    }
                }
            }
        }
    }

    // Handle -W: turn warnings on
    if opts.warnings_on {
        if let Ok(chunk) = vm.compile("warn('@on')") {
            let _ = vm.execute_chunk(Rc::new(chunk));
        }
    }

    // Setup arg table
    let exe_path = env::args().next().unwrap_or_else(|| "lua".to_string());
    setup_arg_table(
        &mut vm,
        &exe_path,
        opts.script_file.as_deref(),
        &opts.script_args,
    );

    // Require modules
    for module in &opts.require_modules {
        if let Err(e) = require_module(&mut vm, module) {
            eprintln!("lua: {}", e);
            return 1;
        }
    }

    // Execute all -e strings in order (they share the same VM state)
    for code in &opts.execute_strings {
        match vm.compile(code) {
            Ok(chunk) => {
                if let Err(e) = vm.execute_chunk(Rc::new(chunk)) {
                    let error_msg = vm.get_error_message(e);
                    let traceback = vm.generate_traceback(&error_msg);
                    eprintln!("lua: Runtime Error: {}", traceback);
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("lua: {}", e);
                return 1;
            }
        }
    }

    // Execute script file or stdin
    if let Some(filename) = &opts.script_file {
        // Set package.path to include the script's directory
        if let Some(parent) = std::path::Path::new(filename).parent() {
            let parent_str = parent.to_string_lossy();
            let dir = if parent_str.is_empty() {
                ".".to_string()
            } else {
                parent_str.to_string()
            };
            // Prepend script dir to package.path
            let set_path = format!(
                "package.path = '{dir}/?.lua;{dir}/?/init.lua;' .. package.path",
                dir = dir.replace('\\', "/")
            );
            if let Ok(chunk) = vm.compile(&set_path) {
                let _ = vm.execute_chunk(Rc::new(chunk));
            }
        }
        if let Err(e) = execute_file(&mut vm, filename) {
            eprintln!("lua: {}", e);
            return 1;
        }
        // for ai debug
        eprintln!();
    } else if opts.read_stdin {
        if let Err(e) = execute_stdin(&mut vm) {
            eprintln!("lua: {}", e);
            return 1;
        }
        // for ai debug
        eprintln!();
    }

    // Enter interactive mode if requested or if no script was provided
    if opts.interactive
        || (opts.execute_strings.is_empty() && opts.script_file.is_none() && !opts.read_stdin)
    {
        run_repl(&mut vm);
    }

    0
}
