// IO library implementation
// Implements: close, flush, input, lines, open, output, read, write, type
mod file;

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaUserdata, LuaValue};
use crate::lua_vm::{LuaResult, LuaState};
pub use file::{LuaFile, create_file_metatable};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};

pub fn create_io_lib() -> LibraryModule {
    crate::lib_module!("io", {
        "write" => io_write,
        "read" => io_read,
        "flush" => io_flush,
        "open" => io_open,
        "lines" => io_lines,
        "input" => io_input,
        "output" => io_output,
        "type" => io_type,
        "tmpfile" => io_tmpfile,
        "close" => io_close,
        "popen" => io_popen,
    })
}

// Note: stdin, stdout, stderr should be initialized separately with init_io_streams()

/// Initialize io standard streams (called after library registration)
pub fn init_io_streams(l: &mut LuaState) -> LuaResult<()> {
    let io_table = l
        .get_global("io")?
        .ok_or_else(|| l.error("io table not found".to_string()))?;

    if !io_table.is_table() {
        return Err(l.error("io must be a table".to_string()));
    };

    // Create stdin
    let stdin_val = create_stdin(l)?;
    let stdin_key = l.create_string("stdin")?;
    l.raw_set(&io_table, stdin_key, stdin_val);

    // Create stdout
    let stdout_val = create_stdout(l)?;
    let stdout_key = l.create_string("stdout")?;
    l.raw_set(&io_table, stdout_key, stdout_val);

    // Create stderr
    let stderr_val = create_stderr(l)?;
    let stderr_key = l.create_string("stderr")?;
    l.raw_set(&io_table, stderr_key, stderr_val);

    Ok(())
}

/// Create stdin file handle
fn create_stdin(l: &mut LuaState) -> LuaResult<LuaValue> {
    let file = LuaFile::stdin();
    let file_mt = create_file_metatable(l)?;
    let userdata = l.create_userdata(LuaUserdata::new(file))?;

    if let Some(ud) = userdata.as_userdata_mut() {
        ud.set_metatable(file_mt);
    }

    // Register userdata for __gc finalization if present
    l.vm_mut().gc.check_finalizer(&userdata);

    Ok(userdata)
}

/// Create stdout file handle
fn create_stdout(l: &mut LuaState) -> LuaResult<LuaValue> {
    let file = LuaFile::stdout();
    let file_mt = create_file_metatable(l)?;
    let userdata = l.create_userdata(LuaUserdata::new(file))?;

    if let Some(ud) = userdata.as_userdata_mut() {
        ud.set_metatable(file_mt);
    }

    // Register userdata for __gc finalization if present
    l.vm_mut().gc.check_finalizer(&userdata);

    Ok(userdata)
}

/// Create stderr file handle
fn create_stderr(l: &mut LuaState) -> LuaResult<LuaValue> {
    let file = LuaFile::stderr();
    let file_mt = create_file_metatable(l)?;
    let userdata = l.create_userdata(LuaUserdata::new(file))?;

    if let Some(ud) = userdata.as_userdata_mut() {
        ud.set_metatable(file_mt);
    }

    // Register userdata for __gc finalization if present
    l.vm_mut().gc.check_finalizer(&userdata);

    Ok(userdata)
}

/// io.write(...) - Write to default output file
fn io_write(l: &mut LuaState) -> LuaResult<usize> {
    // Get default output file from registry
    let registry = l.vm_mut().registry.clone();
    let key = l.create_string("_IO_output")?;

    let output_file = if let Some(registry_table) = registry.as_table() {
        registry_table.raw_get(&key)
    } else {
        None
    };

    // If no output set, use stdout
    let file_handle = if let Some(output) = output_file {
        output
    } else {
        let io_table = l
            .get_global("io")?
            .ok_or_else(|| l.error("io not found".to_string()))?;
        let stdout_key = l.create_string("stdout")?;

        if let Some(io_tbl) = io_table.as_table() {
            io_tbl
                .raw_get(&stdout_key)
                .ok_or_else(|| l.error("stdout not found".to_string()))?
        } else {
            return Err(l.error("io table is not a table".to_string()));
        }
    };

    // Get the file from userdata
    if let Some(ud) = file_handle.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            // Write all arguments
            let mut i = 1;
            loop {
                let arg = match l.get_arg(i) {
                    Some(v) => v,
                    None => break,
                };

                let text = if let Some(s) = arg.as_str() {
                    s.to_string()
                } else if let Some(n) = arg.as_number() {
                    n.to_string()
                } else {
                    return Err(
                        l.error("bad argument to 'write' (string or number expected)".to_string())
                    );
                };

                if let Err(e) = lua_file.write(&text) {
                    return Err(l.error(format!("write error: {}", e)));
                }

                i += 1;
            }

            // Return the file handle
            l.push_value(file_handle)?;
            return Ok(1);
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// io.read([format]) - Read from stdin
fn io_read(l: &mut LuaState) -> LuaResult<usize> {
    let format = l.get_arg(1);

    let stdin = io::stdin();
    let mut handle = stdin.lock();

    // Default to "*l" (read line)
    let format_str = format
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "*l".to_string());

    let result = match format_str.as_str() {
        "*l" | "*L" => {
            // Read a line
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => LuaValue::nil(), // EOF
                Ok(_) => {
                    // Remove trailing newline if present
                    if format_str == "*l" && line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    l.create_string(&line)?
                }
                Err(e) => return Err(l.error(format!("read error: {}", e))),
            }
        }
        "*a" => {
            // Read all
            let mut content = String::new();
            match io::Read::read_to_string(&mut handle, &mut content) {
                Ok(_) => l.create_string(&content)?,
                Err(e) => return Err(l.error(format!("read error: {}", e))),
            }
        }
        "*n" => {
            // Read a number
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => LuaValue::nil(), // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if let Ok(n) = trimmed.parse::<i64>() {
                        LuaValue::integer(n)
                    } else if let Ok(n) = trimmed.parse::<f64>() {
                        LuaValue::float(n)
                    } else {
                        LuaValue::nil()
                    }
                }
                Err(e) => return Err(l.error(format!("read error: {}", e))),
            }
        }
        _ => {
            // Try to parse as number (bytes to read)
            if let Ok(n) = format_str.parse::<usize>() {
                let mut buffer = vec![0u8; n];
                match io::Read::read(&mut handle, &mut buffer) {
                    Ok(0) => LuaValue::nil(), // EOF
                    Ok(bytes_read) => {
                        buffer.truncate(bytes_read);
                        l.create_string(&String::from_utf8_lossy(&buffer))?
                    }
                    Err(e) => return Err(l.error(format!("read error: {}", e))),
                }
            } else {
                return Err(l.error(format!(
                    "bad argument to 'read' (invalid format '{}')",
                    format_str
                )));
            }
        }
    };

    l.push_value(result)?;
    Ok(1)
}

/// io.flush() - Flush stdout
fn io_flush(_l: &mut LuaState) -> LuaResult<usize> {
    io::stdout().flush().ok();
    Ok(0)
}

/// io.open(filename [, mode]) - Open a file
fn io_open(l: &mut LuaState) -> LuaResult<usize> {
    let filename_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'io.open' (string expected)".to_string()))?;
    let filename_str = match filename_val.as_str() {
        Some(s) => s.to_string(),
        None => return Err(l.error("bad argument #1 to 'io.open' (string expected)".to_string())),
    };

    let mode_str = l
        .get_arg(2)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "r".to_string());
    let mode = mode_str.as_str();

    let file_result = match mode {
        "r" => LuaFile::open_read(&filename_str),
        "w" => LuaFile::open_write(&filename_str),
        "a" => LuaFile::open_append(&filename_str),
        "r+" | "w+" | "a+" => LuaFile::open_readwrite(&filename_str),
        _ => return Err(l.error(format!("invalid mode: {}", mode))),
    };

    match file_result {
        Ok(file) => {
            // Create file metatable
            let file_mt = create_file_metatable(l)?;

            // Create userdata
            let userdata = l.create_userdata(LuaUserdata::new(file))?;

            // Set metatable
            if let Some(ud) = userdata.as_userdata_mut() {
                ud.set_metatable(file_mt);
            }

            // Register userdata for __gc finalization if present
            l.vm_mut().gc.check_finalizer(&userdata);

            // Register userdata for __gc finalization if present
            l.vm_mut().gc.check_finalizer(&userdata);

            l.push_value(userdata)?;
            Ok(1)
        }
        Err(e) => {
            // Return nil and error message
            l.push_value(LuaValue::nil())?;
            let err_str = l.create_string(&e.to_string())?;
            l.push_value(err_str)?;
            Ok(2)
        }
    }
}

/// io.lines([filename]) - Return iterator for lines
fn io_lines(l: &mut LuaState) -> LuaResult<usize> {
    let filename = l.get_arg(1);

    if let Some(filename_val) = filename {
        // io.lines(filename) - open file and return iterator
        let filename_str = match filename_val.as_str() {
            Some(s) => s.to_string(),
            None => return Err(l.error("bad argument #1 to 'lines' (string expected)".to_string())),
        };

        // Open the file
        match LuaFile::open_read(&filename_str) {
            Ok(file) => {
                // Create file metatable
                let file_mt = create_file_metatable(l)?;

                // Create userdata
                let userdata = l.create_userdata(LuaUserdata::new(file))?;

                // Set metatable
                if let Some(ud) = userdata.as_userdata_mut() {
                    ud.set_metatable(file_mt);
                }

                // Register userdata for __gc finalization if present
                l.vm_mut().gc.check_finalizer(&userdata);

                // Create state table with file handle
                let state_table = l.create_table(0, 1)?;
                let file_key = l.create_string("file")?;
                l.raw_set(&state_table, file_key, userdata);

                l.push_value(LuaValue::cfunction(io_lines_iterator))?;
                l.push_value(state_table)?;
                l.push_value(LuaValue::nil())?;
                Ok(3)
            }
            Err(e) => Err(l.error(format!("cannot open file '{}': {}", filename_str, e))),
        }
    } else {
        // io.lines() - read from stdin
        Err(l.error("io.lines() without filename not yet implemented".to_string()))
    }
}

/// Iterator function for io.lines()
fn io_lines_iterator(l: &mut LuaState) -> LuaResult<usize> {
    let state_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("iterator requires state".to_string()))?;
    let file_key = l.create_string("file")?;
    let file_val = l
        .raw_get(&state_val, &file_key)
        .ok_or_else(|| l.error("file not found in state".to_string()))?;

    // Read next line
    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            let res = lua_file.read_line();
            match res {
                Ok(Some(line)) => {
                    let line_str: LuaValue = l.create_string(&line)?;
                    l.push_value(line_str)?;
                    return Ok(1);
                }
                Ok(None) => {
                    l.push_value(LuaValue::nil())?;
                    return Ok(1);
                }
                Err(e) => return Err(l.error(format!("read error: {}", e))),
            }
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// io.input([file]) - Set or get default input file
fn io_input(l: &mut LuaState) -> LuaResult<usize> {
    let arg = l.get_arg(1);

    if let Some(arg_val) = arg {
        // Set new input file
        if let Some(filename) = arg_val.as_str() {
            // Open file for reading
            let file = match std::fs::File::open(filename) {
                Ok(f) => f,
                Err(e) => {
                    return Err(l.error(format!("cannot open file '{}': {}", filename, e)));
                }
            };

            let lua_file = LuaFile::from_file(file);
            let file_mt = create_file_metatable(l)?;
            let userdata = l.create_userdata(LuaUserdata::new(lua_file))?;

            if let Some(ud) = userdata.as_userdata_mut() {
                ud.set_metatable(file_mt);
            }

            l.vm_mut().gc.check_finalizer(&userdata);

            // Store in registry
            let registry = l.vm_mut().registry.clone();
            let key = l.create_string("_IO_input")?;
            l.raw_set(&registry, key, userdata);
        } else if arg_val.is_userdata() {
            // Verify it's a valid file handle
            if let Some(ud) = arg_val.as_userdata_mut() {
                let data = ud.get_data_mut();
                if data.downcast_ref::<LuaFile>().is_none() {
                    return Err(l.error("bad argument #1 to 'input' (file expected)".to_string()));
                }
            }

            // Store in registry
            let registry = l.vm_mut().registry.clone();
            let key = l.create_string("_IO_input")?;
            l.raw_set(&registry, key, arg_val);
        } else {
            return Err(l.error("bad argument #1 to 'input' (string or file expected)".to_string()));
        }
    }

    // Return current input file
    let registry = l.vm_mut().registry.clone();
    let key = l.create_string("_IO_input")?;

    if let Some(registry_table) = registry.as_table() {
        if let Some(input) = registry_table.raw_get(&key) {
            l.push_value(input)?;
            return Ok(1);
        }
    }

    // If no input set, return stdin
    let io_table = l
        .get_global("io")?
        .ok_or_else(|| l.error("io not found".to_string()))?;
    let stdin_key = l.create_string("stdin")?;

    if let Some(io_tbl) = io_table.as_table() {
        if let Some(stdin) = io_tbl.raw_get(&stdin_key) {
            l.push_value(stdin)?;
            return Ok(1);
        }
    }

    Err(l.error("stdin not found".to_string()))
}

/// io.output([file]) - Set or get default output file
fn io_output(l: &mut LuaState) -> LuaResult<usize> {
    let arg = l.get_arg(1);

    if let Some(arg_val) = arg {
        // Set new output file
        if let Some(filename) = arg_val.as_str() {
            // Create parent directories if they don't exist
            if let Some(parent) = std::path::Path::new(filename).parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }

            // Open file for writing
            let file = match std::fs::File::create(filename) {
                Ok(f) => f,
                Err(e) => {
                    return Err(l.error(format!("cannot open file '{}': {}", filename, e)));
                }
            };

            let lua_file = LuaFile::from_file(file);
            let file_mt = create_file_metatable(l)?;
            let userdata = l.create_userdata(LuaUserdata::new(lua_file))?;

            if let Some(ud) = userdata.as_userdata_mut() {
                ud.set_metatable(file_mt);
            }

            l.vm_mut().gc.check_finalizer(&userdata);

            // Store in registry
            let registry = l.vm_mut().registry.clone();
            let key = l.create_string("_IO_output")?;
            l.raw_set(&registry, key, userdata);
        } else if arg_val.is_userdata() {
            // Verify it's a valid file handle
            if let Some(ud) = arg_val.as_userdata_mut() {
                let data = ud.get_data_mut();
                if data.downcast_ref::<LuaFile>().is_none() {
                    return Err(l.error("bad argument #1 to 'output' (file expected)".to_string()));
                }
            }

            // Store in registry
            let registry = l.vm_mut().registry.clone();
            let key = l.create_string("_IO_output")?;
            l.raw_set(&registry, key, arg_val);
        } else {
            return Err(
                l.error("bad argument #1 to 'output' (string or file expected)".to_string())
            );
        }
    }

    // Return current output file
    let registry = l.vm_mut().registry.clone();
    let key = l.create_string("_IO_output")?;

    if let Some(registry_table) = registry.as_table() {
        if let Some(output) = registry_table.raw_get(&key) {
            l.push_value(output)?;
            return Ok(1);
        }
    }

    // If no output set, return stdout
    let io_table = l
        .get_global("io")?
        .ok_or_else(|| l.error("io not found".to_string()))?;
    let stdout_key = l.create_string("stdout")?;

    if let Some(io_tbl) = io_table.as_table() {
        if let Some(stdout) = io_tbl.raw_get(&stdout_key) {
            l.push_value(stdout)?;
            return Ok(1);
        }
    }

    Err(l.error("stdout not found".to_string()))
}

/// io.type(obj) - Check if obj is a file handle
fn io_type(l: &mut LuaState) -> LuaResult<usize> {
    let obj = l.get_arg(1);

    if let Some(val) = obj {
        if let Some(ud) = val.as_userdata_mut() {
            let data = ud.get_data_mut();
            if let Some(lua_file) = data.downcast_ref::<LuaFile>() {
                if lua_file.is_closed() {
                    let result = l.create_string("closed file")?;
                    l.push_value(result)?;
                    return Ok(1);
                } else {
                    let result = l.create_string("file")?;
                    l.push_value(result)?;
                    return Ok(1);
                }
            }
        }
    }

    l.push_value(LuaValue::nil())?;
    Ok(1)
}

/// io.tmpfile() - Create a temporary file
fn io_tmpfile(l: &mut LuaState) -> LuaResult<usize> {
    // Create a temporary file manually without external dependencies
    // Use system temp directory + random name
    let temp_dir = std::env::temp_dir();

    // Generate a unique filename using timestamp and process ID
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let filename = format!("lua_tmp_{}_{}.tmp", pid, timestamp);
    let temp_path = temp_dir.join(filename);

    // Open with read+write, create new, delete on close (platform-specific)
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp_path)
    {
        Ok(file) => {
            // On success, try to delete the file immediately
            // On Unix, the file remains accessible via the open handle
            // On Windows, we'll need to delete it on close
            #[cfg(unix)]
            let _ = std::fs::remove_file(&temp_path);

            // Wrap in LuaFile
            let lua_file = LuaFile::from_file(file);

            // Create file metatable
            let file_mt = create_file_metatable(l)?;

            // Create userdata
            let userdata = l.create_userdata(LuaUserdata::new(lua_file))?;

            // Set metatable
            if let Some(ud) = userdata.as_userdata_mut() {
                ud.set_metatable(file_mt);
            }

            l.push_value(userdata)?;
            Ok(1)
        }
        Err(e) => {
            l.push_value(LuaValue::nil())?;
            let err_str = l.create_string(&e.to_string())?;
            l.push_value(err_str)?;
            Ok(2)
        }
    }
}

/// io.close([file]) - Close a file
fn io_close(l: &mut LuaState) -> LuaResult<usize> {
    let file_arg = l.get_arg(1);

    let file_val = if let Some(file) = file_arg {
        file
    } else {
        // No file given - close default output
        let registry = l.vm_mut().registry.clone();
        let key = l.create_string("_IO_output")?;

        if let Some(registry_table) = registry.as_table() {
            registry_table
                .raw_get(&key)
                .ok_or_else(|| l.error("no default output file".to_string()))?
        } else {
            return Err(l.error("registry is not a table".to_string()));
        }
    };

    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            // Don't actually close standard streams
            if lua_file.is_std_stream() {
                l.push_value(LuaValue::boolean(true))?;
                return Ok(1);
            }
            match lua_file.close() {
                Ok(_) => {
                    l.push_value(LuaValue::boolean(true))?;
                    return Ok(1);
                }
                Err(e) => return Err(l.error(format!("close error: {}", e))),
            }
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// io.popen(prog [, mode]) - Execute program and return file handle
fn io_popen(l: &mut LuaState) -> LuaResult<usize> {
    // io.popen is platform-specific and potentially dangerous
    // Stub for now
    Err(l.error("io.popen not yet implemented".to_string()))
}
