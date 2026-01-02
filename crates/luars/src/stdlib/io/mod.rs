// IO library implementation
// Implements: close, flush, input, lines, open, output, read, write, type

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaUserdata, LuaValue};
use crate::lua_vm::{LuaResult, LuaState};
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
        "type" => io_type,
        "tmpfile" => io_tmpfile,
        "close" => io_close,
        "popen" => io_popen,
    })
}

// Note: stdin, stdout, stderr should be initialized separately with init_io_streams()

/// Initialize io standard streams (called after library registration)
pub fn init_io_streams(l: &mut LuaState) -> LuaResult<()> {
    let io_table = l.get_global("io")
        .ok_or_else(|| l.error("io table not found".to_string()))?;
    
    let Some(io_id) = io_table.as_table_id() else {
        return Err(l.error("io must be a table".to_string()));
    };

    // Create stdin
    let stdin_val = create_stdin(l)?;
    let stdin_key = l.create_string("stdin");
    {
        let vm = l.vm_mut();
        let Some(io_tbl) = vm.object_pool.get_table_mut(io_id) else {
            return Err(l.error("io table not found".to_string()));
        };
        io_tbl.raw_set(stdin_key, stdin_val);
    }

    // Create stdout
    let stdout_val = create_stdout(l)?;
    let stdout_key = l.create_string("stdout");
    {
        let vm = l.vm_mut();
        let Some(io_tbl) = vm.object_pool.get_table_mut(io_id) else {
            return Err(l.error("io table not found".to_string()));
        };
        io_tbl.raw_set(stdout_key, stdout_val);
    }

    // Create stderr
    let stderr_val = create_stderr(l)?;
    let stderr_key = l.create_string("stderr");
    {
        let vm = l.vm_mut();
        let Some(io_tbl) = vm.object_pool.get_table_mut(io_id) else {
            return Err(l.error("io table not found".to_string()));
        };
        io_tbl.raw_set(stderr_key, stderr_val);
    }

    Ok(())
}

/// Create stdin file handle
fn create_stdin(l: &mut LuaState) -> LuaResult<LuaValue> {
    let file = LuaFile::stdin();
    let file_mt = create_file_metatable(l)?;
    let userdata = l.create_userdata(LuaUserdata::new(file));
    if let Some(ud_id) = userdata.as_userdata_id() {
        if let Some(ud) = l.get_userdata_mut(&userdata) {
            ud.set_metatable(file_mt);
        }
    }
    Ok(userdata)
}

/// Create stdout file handle
fn create_stdout(l: &mut LuaState) -> LuaResult<LuaValue> {
    let file = LuaFile::stdout();
    let file_mt = create_file_metatable(l)?;
    let userdata = l.create_userdata(LuaUserdata::new(file));
    if let Some(ud_id) = userdata.as_userdata_id() {
        if let Some(ud) = l.get_userdata_mut(&userdata) {
            ud.set_metatable(file_mt);
        }
    }
    Ok(userdata)
}

/// Create stderr file handle
fn create_stderr(l: &mut LuaState) -> LuaResult<LuaValue> {
    let file = LuaFile::stderr();
    let file_mt = create_file_metatable(l)?;
    let userdata = l.create_userdata(LuaUserdata::new(file));
    if let Some(ud_id) = userdata.as_userdata_id() {
        if let Some(ud) = l.get_userdata_mut(&userdata) {
            ud.set_metatable(file_mt);
        }
    }
    Ok(userdata)
}

/// io.write(...) - Write to stdout
fn io_write(l: &mut LuaState) -> LuaResult<usize> {
    let mut i = 1;
    loop {
        let arg = match l.get_arg(i) {
            Some(v) => v,
            None => break,
        };

        if let Some(s) = l.get_string(&arg) {
            print!("{}", s.as_str());
        } else if let Some(n) = arg.as_number() {
            print!("{}", n);
        } else {
            return Err(l.error("bad argument to 'write' (string or number expected)".to_string()));
        }
        i += 1;
    }

    Ok(0)
}

/// io.read([format]) - Read from stdin
fn io_read(l: &mut LuaState) -> LuaResult<usize> {
    let format = l.get_arg(1);

    let stdin = io::stdin();
    let mut handle = stdin.lock();

    // Default to "*l" (read line)
    let format_str = format
        .and_then(|v| l.get_string(&v).map(|s| s.as_str().to_string()))
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
                    l.create_string(&line)
                }
                Err(e) => return Err(l.error(format!("read error: {}", e))),
            }
        }
        "*a" => {
            // Read all
            let mut content = String::new();
            match io::Read::read_to_string(&mut handle, &mut content) {
                Ok(_) => l.create_string(&content),
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
                        l.create_string(&String::from_utf8_lossy(&buffer))
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
    let filename_val = l.get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'io.open' (string expected)".to_string()))?;
    let filename_str = match l.get_string(&filename_val) {
        Some(s) => s.as_str().to_string(),
        None => return Err(l.error("bad argument #1 to 'io.open' (string expected)".to_string())),
    };

    let mode_str = l.get_arg(2)
        .and_then(|v| l.get_string(&v).map(|s| s.as_str().to_string()))
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
            let userdata = l.create_userdata(LuaUserdata::new(file));

            // Set metatable
            if let Some(_ud_id) = userdata.as_userdata_id() {
                if let Some(ud) = l.get_userdata_mut(&userdata) {
                    ud.set_metatable(file_mt);
                }
            }

            l.push_value(userdata)?;
            Ok(1)
        }
        Err(e) => {
            // Return nil and error message
            l.push_value(LuaValue::nil())?;
            let err_str = l.create_string(&e.to_string());
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
        let filename_str = match l.get_string(&filename_val) {
            Some(s) => s.as_str().to_string(),
            None => return Err(l.error("bad argument #1 to 'lines' (string expected)".to_string())),
        };

        // Open the file
        match LuaFile::open_read(&filename_str) {
            Ok(file) => {
                // Create file metatable
                let file_mt = create_file_metatable(l)?;

                // Create userdata
                let userdata = l.create_userdata(LuaUserdata::new(file));

                // Set metatable
                if let Some(_ud_id) = userdata.as_userdata_id() {
                    if let Some(ud) = l.get_userdata_mut(&userdata) {
                        ud.set_metatable(file_mt);
                    }
                }

                // Create state table with file handle
                let state_table = l.create_table(0, 1);
                let file_key = l.create_string("file");
                l.table_set(&state_table, file_key, userdata);

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
    let state_val = l.get_arg(1)
        .ok_or_else(|| l.error("iterator requires state".to_string()))?;
    let file_key = l.create_string("file");
    let file_val = l.table_get(&state_val, &file_key)
        .ok_or_else(|| l.error("file not found in state".to_string()))?;
    
    // Read next line
    if let Some(ud) = l.get_userdata(&file_val) {
        let data = ud.get_data();
        let mut data_ref = data.borrow_mut();
        if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
            let res = lua_file.read_line();
            match res {
                Ok(Some(line)) => {
                    let line_str: LuaValue = l.create_string(&line);
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
    // Stub: would need file handle support
    Err(l.error("io.input not yet implemented".to_string()))
}

/// io.output([file]) - Set or get default output file
fn io_output(l: &mut LuaState) -> LuaResult<usize> {
    // Stub: would need file handle support
    Err(l.error("io.output not yet implemented".to_string()))
}

/// io.type(obj) - Check if obj is a file handle
fn io_type(l: &mut LuaState) -> LuaResult<usize> {
    let obj = l.get_arg(1);

    if let Some(val) = obj {
        if let Some(ud) = l.get_userdata(&val) {
            let data = ud.get_data();
            let data_ref = data.borrow();
            if data_ref.downcast_ref::<LuaFile>().is_some() {
                // Check if file is closed
                drop(data_ref);
                let data_ref = data.borrow();
                if let Some(lua_file) = data_ref.downcast_ref::<LuaFile>() {
                    if lua_file.is_closed() {
                        let result = l.create_string("closed file");
                        l.push_value(result)?;
                        return Ok(1);
                    } else {
                        let result = l.create_string("file");
                        l.push_value(result)?;
                        return Ok(1);
                    }
                }
            }
        }
    }

    l.push_value(LuaValue::nil())?;
    Ok(1)
}

/// io.tmpfile() - Create a temporary file
fn io_tmpfile(l: &mut LuaState) -> LuaResult<usize> {
    // Create a temporary file
    match tempfile::tempfile() {
        Ok(file) => {
            // Wrap in LuaFile
            let lua_file = LuaFile::from_file(file);

            // Create file metatable
            let file_mt = create_file_metatable(l)?;

            // Create userdata
            let userdata = l.create_userdata(LuaUserdata::new(lua_file));

            // Set metatable
            if let Some(_ud_id) = userdata.as_userdata_id() {
                if let Some(ud) = l.get_userdata_mut(&userdata) {
                    ud.set_metatable(file_mt);
                }
            }

            l.push_value(userdata)?;
            Ok(1)
        }
        Err(e) => {
            l.push_value(LuaValue::nil())?;
            let err_str = l.create_string(&e.to_string());
            l.push_value(err_str)?;
            Ok(2)
        }
    }
}

/// io.close([file]) - Close a file
fn io_close(l: &mut LuaState) -> LuaResult<usize> {
    let file_arg = l.get_arg(1);

    if let Some(file_val) = file_arg {
        if let Some(ud) = l.get_userdata(&file_val) {
            let data = ud.get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
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
    }

    // No file given - close default output (stub)
    Err(l.error("io.close without argument not yet implemented".to_string()))
}

/// io.popen(prog [, mode]) - Execute program and return file handle
fn io_popen(l: &mut LuaState) -> LuaResult<usize> {
    // io.popen is platform-specific and potentially dangerous
    // Stub for now
    Err(l.error("io.popen not yet implemented".to_string()))
}
