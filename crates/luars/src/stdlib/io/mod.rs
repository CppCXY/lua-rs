// IO library implementation
// Implements: close, flush, input, lines, open, output, read, write, type

use crate::lib_registry::{get_arg, get_args, require_arg, LibraryModule};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};
use std::io::{self, BufRead, Write};

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
fn io_write(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    for arg in args {
        if let Some(s) = vm.get_string(&arg) {
            print!("{}", s.as_str());
        } else if let Some(n) = arg.as_number() {
            print!("{}", n);
        } else {
            return Err(vm.error("bad argument to 'write' (string or number expected)"));
        }
    }

    Ok(MultiValue::empty())
}

/// io.read([format]) - Read from stdin
fn io_read(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let format = get_arg(vm, 1);

    let stdin = io::stdin();
    let mut handle = stdin.lock();

    // Default to "*l" (read line)
    let format_str = format
        .and_then(|v| vm.get_string(&v).map(|s| s.as_str().to_string()))
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
                    Ok(MultiValue::single(vm.create_string(&line)))
                }
                Err(e) => Err(vm.error(format!("read error: {}", e))),
            }
        }
        "*a" => {
            // Read all
            let mut content = String::new();
            match io::Read::read_to_string(&mut handle, &mut content) {
                Ok(_) => Ok(MultiValue::single(vm.create_string(&content))),
                Err(e) => Err(vm.error(format!("read error: {}", e))),
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
                Err(e) => Err(vm.error(format!("read error: {}", e))),
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
                        Ok(MultiValue::single(
                            vm.create_string(&String::from_utf8_lossy(&buffer)),
                        ))
                    }
                    Err(e) => Err(vm.error(format!("read error: {}", e))),
                }
            } else {
                Err(vm.error(format!(
                    "bad argument to 'read' (invalid format '{}')",
                    format_str
                )))
            }
        }
    }
}

/// io.flush() - Flush stdout
fn io_flush(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    io::stdout().flush().ok();
    Ok(MultiValue::empty())
}

/// io.open(filename [, mode]) - Open a file
fn io_open(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let filename_val = require_arg(vm, 1, "io.open")?;
    let filename_str = match vm.get_string(&filename_val) {
        Some(s) => s.as_str().to_string(),
        None => return Err(vm.error("bad argument #1 to 'io.open' (string expected)")),
    };

    let mode_str = get_arg(vm, 2)
        .and_then(|v| vm.get_string(&v).map(|s| s.as_str().to_string()))
        .unwrap_or_else(|| "r".to_string());
    let mode = mode_str.as_str();

    let file_result = match mode {
        "r" => LuaFile::open_read(&filename_str),
        "w" => LuaFile::open_write(&filename_str),
        "a" => LuaFile::open_append(&filename_str),
        "r+" | "w+" | "a+" => LuaFile::open_readwrite(&filename_str),
        _ => return Err(vm.error(format!("invalid mode: {}", mode))),
    };

    match file_result {
        Ok(file) => {
            // Create file metatable if not already created
            let file_mt = create_file_metatable(vm)?;

            // Create userdata with VM (proper GC tracking)
            use crate::lua_value::LuaUserdata;
            let userdata = vm.create_userdata(LuaUserdata::new(file));

            // Set metatable
            if let Some(ud_id) = userdata.as_userdata_id() {
                if let Some(ud) = vm.object_pool.get_userdata_mut(ud_id) {
                    ud.set_metatable(file_mt);
                }
            }

            Ok(MultiValue::single(userdata))
        }
        Err(e) => {
            // Return nil and error message
            Ok(MultiValue::multiple(vec![
                LuaValue::nil(),
                vm.create_string(&e.to_string()),
            ]))
        }
    }
}

/// io.lines([filename]) - Return iterator for lines
fn io_lines(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Stub: would need to return an iterator function
    Err(vm.error("io.lines not yet implemented"))
}

/// io.input([file]) - Set or get default input file
fn io_input(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Stub: would need file handle support
    Err(vm.error("io.input not yet implemented"))
}

/// io.output([file]) - Set or get default output file
fn io_output(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Stub: would need file handle support
    Err(vm.error("io.output not yet implemented"))
}
