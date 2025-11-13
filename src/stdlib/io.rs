// IO library implementation
// Implements: close, flush, input, lines, open, output, read, write, type

use crate::lib_registry::LibraryModule;
use crate::value::{LuaValue, LuaString, MultiValue};
use crate::vm::VM;
use std::io::{self, BufRead, Write};
use std::rc::Rc;

pub fn create_io_lib() -> LibraryModule {
    crate::lib_module!("io", {
        "write" => io_write,
        "read" => io_read,
        "flush" => io_flush,
        "lines" => io_lines,
        "input" => io_input,
        "output" => io_output,
    })
}

/// io.write(...) - Write to stdout
fn io_write(vm: &mut VM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);
    
    for arg in args {
        if let Some(s) = arg.as_string() {
            print!("{}", s.as_str());
        } else if let Some(n) = arg.as_number() {
            print!("{}", n);
        } else {
            return Err("bad argument to 'write' (string or number expected)".to_string());
        }
    }
    
    Ok(MultiValue::empty())
}

/// io.read([format]) - Read from stdin
fn io_read(vm: &mut VM) -> Result<MultiValue, String> {
    let format = crate::lib_registry::get_arg(vm, 0);
    
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    
    // Default to "*l" (read line)
    let format_str = format
        .and_then(|v| v.as_string().map(|s| s.as_str().to_string()))
        .unwrap_or_else(|| "*l".to_string());
    
    match format_str.as_str() {
        "*l" | "*L" => {
            // Read a line
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => Ok(MultiValue::single(LuaValue::Nil)), // EOF
                Ok(_) => {
                    // Remove trailing newline if present
                    if format_str == "*l" && line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    Ok(MultiValue::single(LuaValue::String(Rc::new(
                        crate::value::LuaString::new(line)
                    ))))
                }
                Err(e) => Err(format!("read error: {}", e)),
            }
        }
        "*a" => {
            // Read all
            let mut content = String::new();
            match io::Read::read_to_string(&mut handle, &mut content) {
                Ok(_) => Ok(MultiValue::single(LuaValue::String(Rc::new(
                    crate::value::LuaString::new(content)
                )))),
                Err(e) => Err(format!("read error: {}", e)),
            }
        }
        "*n" => {
            // Read a number
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => Ok(MultiValue::single(LuaValue::Nil)), // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if let Ok(n) = trimmed.parse::<i64>() {
                        Ok(MultiValue::single(LuaValue::Integer(n)))
                    } else if let Ok(n) = trimmed.parse::<f64>() {
                        Ok(MultiValue::single(LuaValue::Float(n)))
                    } else {
                        Ok(MultiValue::single(LuaValue::Nil))
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
                    Ok(0) => Ok(MultiValue::single(LuaValue::Nil)), // EOF
                    Ok(bytes_read) => {
                        buffer.truncate(bytes_read);
                        Ok(MultiValue::single(LuaValue::String(Rc::new(
                            crate::value::LuaString::new(
                                String::from_utf8_lossy(&buffer).to_string()
                            )
                        ))))
                    }
                    Err(e) => Err(format!("read error: {}", e)),
                }
            } else {
                Err(format!("bad argument to 'read' (invalid format '{}')", format_str))
            }
        }
    }
}

/// io.flush() - Flush stdout
fn io_flush(_vm: &mut VM) -> Result<MultiValue, String> {
    io::stdout().flush().ok();
    Ok(MultiValue::empty())
}

/// io.lines([filename]) - Return iterator for lines
fn io_lines(_vm: &mut VM) -> Result<MultiValue, String> {
    // Stub: would need to return an iterator function
    Err("io.lines not yet implemented".to_string())
}

/// io.input([file]) - Set or get default input file
fn io_input(_vm: &mut VM) -> Result<MultiValue, String> {
    // Stub: would need file handle support
    Err("io.input not yet implemented".to_string())
}

/// io.output([file]) - Set or get default output file
fn io_output(_vm: &mut VM) -> Result<MultiValue, String> {
    // Stub: would need file handle support
    Err("io.output not yet implemented".to_string())
}


