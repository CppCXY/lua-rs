// IO library implementation
// Implements: close, flush, input, lines, open, output, read, write, type

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;
use crate::{LuaString, LuaTable};
use std::cell::RefCell;
use std::io::{self, BufRead, Write};
use std::rc::Rc;

mod file;
pub use file::{LuaFile, create_file_metatable};

pub fn create_io_lib() -> LibraryModule {
    crate::lib_module!("io", {
        "write" => io_write,
        "read" => io_read,
        "flush" => io_flush,
        "open" => io_open,
        "lines" => io_lines,
        "input" => io_input,
        "output" => io_output,
    })
}

/// io.write(...) - Write to stdout
fn io_write(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);

    for arg in args {
        unsafe {
            if let Some(s) = arg.as_string() {
                print!("{}", s.as_str());
            } else if let Some(n) = arg.as_number() {
                print!("{}", n);
            } else {
                return Err("bad argument to 'write' (string or number expected)".to_string());
            }
        }
    }

    Ok(MultiValue::empty())
}

/// io.read([format]) - Read from stdin
fn io_read(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let format = crate::lib_registry::get_arg(vm, 0);

    let stdin = io::stdin();
    let mut handle = stdin.lock();

    // Default to "*l" (read line)
    let format_str = format
        .and_then(|v| v.as_string_rc().map(|s| s.as_str().to_string()))
        .unwrap_or_else(|| "*l".to_string());

    match format_str.as_str() {
        "*l" | "*L" => {
            // Read a line
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => Ok(MultiValue::single(LuaValue::nil())), // EOF
                Ok(_) => {
                    // Remove trailing newline if present
                    if format_str == "*l" && line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    Ok(MultiValue::single(LuaValue::from_string_rc(Rc::new(
                        LuaString::new(line),
                    ))))
                }
                Err(e) => Err(format!("read error: {}", e)),
            }
        }
        "*a" => {
            // Read all
            let mut content = String::new();
            match io::Read::read_to_string(&mut handle, &mut content) {
                Ok(_) => Ok(MultiValue::single(LuaValue::from_string_rc(Rc::new(
                    LuaString::new(content),
                )))),
                Err(e) => Err(format!("read error: {}", e)),
            }
        }
        "*n" => {
            // Read a number
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => Ok(MultiValue::single(LuaValue::nil())), // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if let Ok(n) = trimmed.parse::<i64>() {
                        Ok(MultiValue::single(LuaValue::integer(n)))
                    } else if let Ok(n) = trimmed.parse::<f64>() {
                        Ok(MultiValue::single(LuaValue::float(n)))
                    } else {
                        Ok(MultiValue::single(LuaValue::nil()))
                    }
                }
                Err(e) => Err(format!("read error: {}", e)),
            }
        }
        _ => {
            // Try to parse as number (bytes to read)
            if let Ok(n) = format_str.parse::<usize>() {
                let mut buffer = vec![0u8; n];
                match io::Read::read(&mut handle, &mut buffer) {
                    Ok(0) => Ok(MultiValue::single(LuaValue::nil())), // EOF
                    Ok(bytes_read) => {
                        buffer.truncate(bytes_read);
                        Ok(MultiValue::single(LuaValue::from_string_rc(Rc::new(
                            LuaString::new(String::from_utf8_lossy(&buffer).to_string()),
                        ))))
                    }
                    Err(e) => Err(format!("read error: {}", e)),
                }
            } else {
                Err(format!(
                    "bad argument to 'read' (invalid format '{}')",
                    format_str
                ))
            }
        }
    }
}

/// io.flush() - Flush stdout
fn io_flush(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    io::stdout().flush().ok();
    Ok(MultiValue::empty())
}

/// io.open(filename [, mode]) - Open a file
fn io_open(vm: &mut LuaVM) -> Result<MultiValue, String> {
    use crate::lib_registry::{get_arg, require_arg};

    let filename = require_arg(vm, 0, "io.open")?
        .as_string_rc()
        .ok_or_else(|| "bad argument #1 to 'io.open' (string expected)".to_string())?;

    let mode_str = get_arg(vm, 1)
        .and_then(|v| v.as_string_rc())
        .map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "r".to_string());
    let mode = mode_str.as_str();

    let file_result = match mode {
        "r" => LuaFile::open_read(filename.as_str()),
        "w" => LuaFile::open_write(filename.as_str()),
        "a" => LuaFile::open_append(filename.as_str()),
        "r+" | "w+" | "a+" => LuaFile::open_readwrite(filename.as_str()),
        _ => return Err(format!("invalid mode: {}", mode)),
    };

    match file_result {
        Ok(file) => {
            // Create file metatable if not already created
            let file_mt = create_file_metatable(vm);

            // Convert Rc to raw pointer for GC management
            let mt_ptr = &*file_mt as *const RefCell<LuaTable>;

            // Create userdata with metatable (deprecated but works during migration)
            #[allow(deprecated)]
            let userdata = LuaValue::userdata_with_metatable(file, mt_ptr);

            Ok(MultiValue::single(userdata))
        }
        Err(e) => {
            // Return nil and error message
            Ok(MultiValue::multiple(vec![
                LuaValue::nil(),
                LuaValue::from_string_rc(vm.create_string(e.to_string())),
            ]))
        }
    }
}

/// io.lines([filename]) - Return iterator for lines
fn io_lines(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Stub: would need to return an iterator function
    Err("io.lines not yet implemented".to_string())
}

/// io.input([file]) - Set or get default input file
fn io_input(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Stub: would need file handle support
    Err("io.input not yet implemented".to_string())
}

/// io.output([file]) - Set or get default output file
fn io_output(_vm: &mut LuaVM) -> Result<MultiValue, String> {
    // Stub: would need file handle support
    Err("io.output not yet implemented".to_string())
}
