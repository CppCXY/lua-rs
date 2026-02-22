// IO library implementation
// Implements: close, flush, input, lines, open, output, read, write, type
mod file;

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaUserdata, LuaValue};
use crate::lua_vm::{LuaResult, LuaState};
pub use file::{LuaFile, create_file_metatable};
use std::fs::OpenOptions;
use std::io::{self, Write};

/// Create a LuaValue from raw bytes. If valid UTF-8 and ASCII-only, creates a string.
/// Otherwise creates a binary value to preserve exact byte values.
fn bytes_to_lua_value(l: &mut LuaState, bytes: Vec<u8>) -> LuaResult<LuaValue> {
    match String::from_utf8(bytes.clone()) {
        Ok(s) => l.create_string(&s),
        Err(_) => l.create_binary(bytes),
    }
}

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
    .with_initializer(init_io_streams)
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
/// Helper: get the default output file handle (fast path via VM cache, fallback to registry/io.stdout)
#[inline]
fn get_default_output(l: &mut LuaState) -> LuaResult<LuaValue> {
    // Fast path: use cached handle from VM
    if let Some(handle) = l.vm_mut().io_default_output {
        return Ok(handle);
    }

    // Slow path: look up from registry
    let registry = l.vm_mut().registry;
    let key = l.create_string("_IO_output")?;

    let output_file = if let Some(registry_table) = registry.as_table() {
        registry_table.raw_get(&key)
    } else {
        None
    };

    if let Some(output) = output_file {
        // Cache it for next time
        l.vm_mut().io_default_output = Some(output);
        return Ok(output);
    }

    // Fallback: use io.stdout
    let io_table = l
        .get_global("io")?
        .ok_or_else(|| l.error("io not found".to_string()))?;
    let stdout_key = l.create_string("stdout")?;

    if let Some(io_tbl) = io_table.as_table() {
        let handle = io_tbl
            .raw_get(&stdout_key)
            .ok_or_else(|| l.error("stdout not found".to_string()))?;
        // Cache it
        l.vm_mut().io_default_output = Some(handle);
        Ok(handle)
    } else {
        Err(l.error("io table is not a table".to_string()))
    }
}

/// Helper: get the default input file handle (fast path via VM cache, fallback to registry/io.stdin)
#[inline]
fn get_default_input(l: &mut LuaState) -> LuaResult<LuaValue> {
    // Fast path: use cached handle from VM
    if let Some(handle) = l.vm_mut().io_default_input {
        return Ok(handle);
    }

    // Slow path: look up from registry
    let registry = l.vm_mut().registry;
    let key = l.create_string("_IO_input")?;

    let input_file = if let Some(registry_table) = registry.as_table() {
        registry_table.raw_get(&key)
    } else {
        None
    };

    if let Some(input) = input_file {
        l.vm_mut().io_default_input = Some(input);
        return Ok(input);
    }

    // Fallback: use io.stdin
    let io_table = l
        .get_global("io")?
        .ok_or_else(|| l.error("io not found".to_string()))?;
    let stdin_key = l.create_string("stdin")?;

    if let Some(io_tbl) = io_table.as_table() {
        let handle = io_tbl
            .raw_get(&stdin_key)
            .ok_or_else(|| l.error("stdin not found".to_string()))?;
        l.vm_mut().io_default_input = Some(handle);
        Ok(handle)
    } else {
        Err(l.error("io table is not a table".to_string()))
    }
}

fn io_write(l: &mut LuaState) -> LuaResult<usize> {
    let file_handle = get_default_output(l)?;

    // Get the file from userdata
    if let Some(ud) = file_handle.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            if lua_file.is_closed() {
                return Err(l.error_from_c("default output file is closed".to_string()));
            }
            // Write all arguments
            let mut i = 1;
            while let Some(arg) = l.get_arg(i) {
                let write_result = if let Some(s) = arg.as_str() {
                    lua_file.write(s)
                } else if let Some(n) = arg.as_integer() {
                    lua_file.write(&n.to_string())
                } else if let Some(n) = arg.as_float() {
                    lua_file.write(&n.to_string())
                } else if let Some(b) = arg.as_binary() {
                    lua_file.write_bytes(b)
                } else {
                    return Err(crate::stdlib::debug::arg_typeerror(
                        l,
                        i,
                        "string or number",
                        &arg,
                    ));
                };

                if let Err(e) = write_result {
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

/// io.read([format, ...]) - Read from default input
fn io_read(l: &mut LuaState) -> LuaResult<usize> {
    let file_handle = get_default_input(l)?;

    if let Some(ud) = file_handle.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            if lua_file.is_closed() {
                return Err(l.error("default input file is closed".to_string()));
            }
            // Collect all format arguments
            let mut formats: Vec<LuaValue> = Vec::new();
            let mut i = 1;
            while let Some(v) = l.get_arg(i) {
                formats.push(v);
                i += 1;
            }
            // Default: read a line
            if formats.is_empty() {
                formats.push(LuaValue::nil()); // sentinel for default "l"
            }

            let mut nresults = 0;
            let mut success = true;

            for fmt in &formats {
                if !success {
                    l.push_value(LuaValue::nil())?;
                    nresults += 1;
                    continue;
                }

                let result = read_one_format_file(l, lua_file, fmt)?;
                if result.is_nil() {
                    success = false;
                }
                l.push_value(result)?;
                nresults += 1;
            }

            return Ok(nresults);
        }
    }

    Err(l.error("expected file handle for default input".to_string()))
}

/// Helper function: read one value from a LuaFile using a format specifier.
/// Shared by io.read, io.lines, and file:read.
fn read_one_format_file(
    l: &mut LuaState,
    lua_file: &mut LuaFile,
    fmt: &LuaValue,
) -> LuaResult<LuaValue> {
    use file::ReadNumberResult;

    // Check if format is an integer (byte count)
    if let Some(n) = fmt.as_integer() {
        let n = n as usize;
        if n == 0 {
            // read(0) returns "" if not EOF, nil if EOF
            match lua_file.is_eof() {
                Ok(true) => return Ok(LuaValue::nil()),
                Ok(false) => return l.create_string(""),
                Err(_) => return Ok(LuaValue::nil()),
            }
        }
        match lua_file.read_bytes(n) {
            Ok(bytes) => {
                if bytes.is_empty() {
                    return Ok(LuaValue::nil());
                }
                return bytes_to_lua_value(l, bytes);
            }
            Err(_) => return Ok(LuaValue::nil()),
        }
    }

    // Get format string (default "l" for nil sentinel)
    let format_str = fmt
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "l".to_string());
    let format = format_str.strip_prefix('*').unwrap_or(&format_str);

    let first_char = format.chars().next().unwrap_or('l');
    match first_char {
        'l' => match lua_file.read_line() {
            Ok(Some(line)) => Ok(l.create_string(&line)?),
            Ok(None) => Ok(LuaValue::nil()),
            Err(_) => Ok(LuaValue::nil()),
        },
        'L' => match lua_file.read_line_with_newline() {
            Ok(Some(line)) => Ok(l.create_string(&line)?),
            Ok(None) => Ok(LuaValue::nil()),
            Err(_) => Ok(LuaValue::nil()),
        },
        'a' => match lua_file.read_all() {
            Ok(content) => Ok(bytes_to_lua_value(l, content)?),
            Err(_) => Ok(LuaValue::nil()),
        },
        'n' => match lua_file.read_number() {
            Ok(Some(ReadNumberResult::Integer(n))) => Ok(LuaValue::integer(n)),
            Ok(Some(ReadNumberResult::Float(n))) => Ok(LuaValue::float(n)),
            Ok(None) => Ok(LuaValue::nil()),
            Err(_) => Ok(LuaValue::nil()),
        },
        _ => Err(l.error("invalid format".to_string())),
    }
}

/// io.flush() - Flush stdout
fn io_flush(l: &mut LuaState) -> LuaResult<usize> {
    let file_handle = get_default_output(l)?;

    if let Some(ud) = file_handle.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            if let Err(e) = lua_file.flush() {
                // Return nil, errmsg, errno (like C Lua)
                l.push_value(LuaValue::nil())?;
                let msg = format!("{}", e);
                let errno = e.raw_os_error().unwrap_or(0) as i64;
                let err_str = l.create_string(&msg)?;
                l.push_value(err_str)?;
                l.push_value(LuaValue::integer(errno))?;
                return Ok(3);
            }
            l.push_value(file_handle)?;
            return Ok(1);
        }
    }

    // Fallback: flush stdout directly
    io::stdout().flush().ok();
    l.push_value(LuaValue::boolean(true))?;
    Ok(1)
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

    // Strip 'b' suffix (binary mode, no-op on most platforms)
    let mode = mode_str.trim_end_matches('b');

    let file_result = match mode {
        "r" => LuaFile::open_read(&filename_str),
        "w" => LuaFile::open_write(&filename_str),
        "a" => LuaFile::open_append(&filename_str),
        "r+" => LuaFile::open_readwrite(&filename_str),
        "w+" => LuaFile::open_write_read(&filename_str),
        "a+" => LuaFile::open_append_read(&filename_str),
        _ => return Err(l.error(format!("invalid mode: {}", mode_str))),
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

            l.push_value(userdata)?;
            Ok(1)
        }
        Err(e) => {
            // Return nil, error message, and errno (matching C Lua)
            l.push_value(LuaValue::nil())?;
            let err_msg = format!("{}: {}", filename_str, e);
            let err_str = l.create_string(&err_msg)?;
            l.push_value(err_str)?;
            let errno = e.raw_os_error().unwrap_or(0) as i64;
            l.push_value(LuaValue::integer(errno))?;
            Ok(3)
        }
    }
}

const MAXARGLINE: usize = 250;

/// io.lines([filename]) - Return iterator for lines
fn io_lines(l: &mut LuaState) -> LuaResult<usize> {
    let filename = l.get_arg(1);

    // Collect format arguments (start from arg 2)
    let mut formats: Vec<LuaValue> = Vec::new();
    let mut i = 2;
    while let Some(v) = l.get_arg(i) {
        formats.push(v);
        i += 1;
    }

    if formats.len() > MAXARGLINE {
        return Err(l.error("too many arguments".to_string()));
    }

    // Check if we have a filename (not nil)
    let has_filename = filename.as_ref().is_some_and(|v| !v.is_nil());

    if has_filename {
        let filename_val = filename.unwrap();
        // io.lines(filename, ...) - open file and return iterator
        let filename_str = match filename_val.as_str() {
            Some(s) => s.to_string(),
            None => return Err(l.error("bad argument #1 to 'lines' (string expected)".to_string())),
        };

        match LuaFile::open_read(&filename_str) {
            Ok(file) => {
                let file_mt = create_file_metatable(l)?;
                let userdata = l.create_userdata(LuaUserdata::new(file))?;
                if let Some(ud) = userdata.as_userdata_mut() {
                    ud.set_metatable(file_mt);
                }
                l.vm_mut().gc.check_finalizer(&userdata);

                let state_table = l.create_table(0, 4)?;
                let file_key = l.create_string("file")?;
                l.raw_set(&state_table, file_key, userdata);
                let closed_key = l.create_string("closed")?;
                l.raw_set(&state_table, closed_key, LuaValue::boolean(false));

                // Store formats
                let fmts_table = l.create_table(formats.len(), 0)?;
                for (idx, fmt) in formats.iter().enumerate() {
                    l.raw_seti(&fmts_table, (idx + 1) as i64, *fmt);
                }
                let fmts_key = l.create_string("fmts")?;
                l.raw_set(&state_table, fmts_key, fmts_table);
                let nfmts_key = l.create_string("nfmts")?;
                l.raw_set(
                    &state_table,
                    nfmts_key,
                    LuaValue::integer(formats.len() as i64),
                );

                // Also set __call so the table is directly callable (for load() etc.)
                let mt = l.create_table(0, 1)?;
                let call_key = l.create_string("__call")?;
                l.raw_set(&mt, call_key, LuaValue::cfunction(io_lines_call));
                if let Some(t) = state_table.as_table_mut() {
                    t.set_metatable(Some(mt));
                }

                // Create C closure for the iterator (captures state table as upvalue)
                // This allows both `for l in io.lines(file)` and `local f = io.lines(file); f()` to work
                let vm = l.vm_mut();
                let iterator_closure = vm.create_c_closure(io_lines_next, vec![state_table])?;

                // Return 4 values for generic for: iterator, state, nil, to-be-closed
                l.push_value(iterator_closure)?;
                l.push_value(state_table)?;
                l.push_value(LuaValue::nil())?;
                l.push_value(userdata)?; // to-be-closed file handle
                Ok(4)
            }
            Err(e) => Err(l.error(format!("cannot open file '{}': {}", filename_str, e))),
        }
    } else {
        // io.lines() or io.lines(nil, ...) - read from default input
        let registry = l.vm_mut().registry;
        let key = l.create_string("_IO_input")?;
        let input_file = if let Some(registry_table) = registry.as_table() {
            registry_table
                .raw_get(&key)
                .ok_or_else(|| l.error("default input file is not set".to_string()))?
        } else {
            return Err(l.error("registry is not a table".to_string()));
        };

        let state_table = l.create_table(0, 5)?;
        let file_key = l.create_string("file")?;
        l.raw_set(&state_table, file_key, input_file);
        let closed_key = l.create_string("closed")?;
        l.raw_set(&state_table, closed_key, LuaValue::boolean(false));
        let noclose_key = l.create_string("noclose")?;
        l.raw_set(&state_table, noclose_key, LuaValue::boolean(true));

        // Store formats
        let fmts_table = l.create_table(formats.len(), 0)?;
        for (idx, fmt) in formats.iter().enumerate() {
            l.raw_seti(&fmts_table, (idx + 1) as i64, *fmt);
        }
        let fmts_key = l.create_string("fmts")?;
        l.raw_set(&state_table, fmts_key, fmts_table);
        let nfmts_key = l.create_string("nfmts")?;
        l.raw_set(
            &state_table,
            nfmts_key,
            LuaValue::integer(formats.len() as i64),
        );

        let mt = l.create_table(0, 1)?;
        let call_key = l.create_string("__call")?;
        l.raw_set(&mt, call_key, LuaValue::cfunction(io_lines_call));
        if let Some(t) = state_table.as_table_mut() {
            t.set_metatable(Some(mt));
        }

        // Create C closure for the iterator
        let vm = l.vm_mut();
        let iterator_closure = vm.create_c_closure(io_lines_next, vec![state_table])?;

        // Return 4 values for generic for: iterator, state, nil, nil
        // No to-be-closed for default input (we don't own the file)
        l.push_value(iterator_closure)?;
        l.push_value(state_table)?;
        l.push_value(LuaValue::nil())?;
        l.push_value(LuaValue::nil())?;
        Ok(4)
    }
}

/// Iterator function for io.lines generic for loop
/// When called from generic for: io_lines_next(state_table, control) - state is arg 1
/// When called standalone: f() - state is in upvalue
fn io_lines_next(l: &mut LuaState) -> LuaResult<usize> {
    // Try to get state from arg 1 first (generic for passes it)
    let state_val = match l.get_arg(1) {
        Some(v) if v.is_table() => v,
        _ => {
            // Get from upvalue (standalone call)
            if let Some(frame) = l.current_frame() {
                if let Some(cclosure) = frame.func.as_cclosure() {
                    if let Some(upval) = cclosure.upvalues().first() {
                        *upval
                    } else {
                        return Err(l.error("iterator state not found".to_string()));
                    }
                } else {
                    return Err(l.error("iterator state not found".to_string()));
                }
            } else {
                return Err(l.error("iterator state not found".to_string()));
            }
        }
    };

    io_lines_call_inner(l, &state_val)
}

/// __call metamethod for io.lines iterator table
pub(crate) fn io_lines_call(l: &mut LuaState) -> LuaResult<usize> {
    // arg 1 is the table itself (self), arg 2+ are args from caller
    let state_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("iterator requires state".to_string()))?;

    io_lines_call_inner(l, &state_val)
}

/// Shared implementation for io.lines iteration
fn io_lines_call_inner(l: &mut LuaState, state_val: &LuaValue) -> LuaResult<usize> {
    // Check if already closed
    let closed_key = l.create_string("closed")?;
    let is_closed = l
        .raw_get(state_val, &closed_key)
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);
    if is_closed {
        return Err(l.error("file is already closed".to_string()));
    }

    let file_key = l.create_string("file")?;
    let file_val = l
        .raw_get(state_val, &file_key)
        .ok_or_else(|| l.error("file not found in state".to_string()))?;

    // Get format count
    let nfmts_key = l.create_string("nfmts")?;
    let nfmts = l
        .raw_get(state_val, &nfmts_key)
        .and_then(|v| v.as_integer())
        .unwrap_or(0) as usize;

    // Get formats table
    let fmts_key = l.create_string("fmts")?;
    let fmts_table = l.raw_get(state_val, &fmts_key).unwrap_or_default();

    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            if nfmts == 0 {
                // Default: read a line
                let res = lua_file.read_line();
                match res {
                    Ok(Some(line)) => {
                        let line_str: LuaValue = l.create_string(&line)?;
                        l.push_value(line_str)?;
                        return Ok(1);
                    }
                    Ok(None) => {
                        return io_lines_close_on_eof(l, lua_file, state_val, &closed_key);
                    }
                    Err(e) => return Err(l.error(format!("read error: {}", e))),
                }
            } else {
                // Read using formats
                let mut results = Vec::with_capacity(nfmts);
                for idx in 1..=nfmts {
                    let fmt = if let Some(ft) = fmts_table.as_table() {
                        ft.raw_geti(idx as i64).unwrap_or(LuaValue::nil())
                    } else {
                        LuaValue::nil()
                    };

                    let result = read_one_format_file(l, lua_file, &fmt)?;
                    results.push(result);
                }
                // If the first result is nil, it means EOF
                if results.first().is_none_or(|v| v.is_nil()) {
                    return io_lines_close_on_eof(l, lua_file, state_val, &closed_key);
                }
                let nresults = results.len();
                for r in results {
                    l.push_value(r)?;
                }
                return Ok(nresults);
            }
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// Helper: close file (or not) on EOF and return nil
fn io_lines_close_on_eof(
    l: &mut LuaState,
    lua_file: &mut LuaFile,
    state_val: &LuaValue,
    closed_key: &LuaValue,
) -> LuaResult<usize> {
    let noclose_key = l.create_string("noclose")?;
    let no_close = l
        .raw_get(state_val, &noclose_key)
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);
    if !no_close {
        let _ = lua_file.close();
    }
    if let Some(t) = state_val.as_table_mut() {
        t.raw_set(closed_key, LuaValue::boolean(true));
    }
    l.push_value(LuaValue::nil())?;
    Ok(1)
}

/// Read one value from file using a format specifier
/// io.input([file]) - Set or get default input file
fn io_input(l: &mut LuaState) -> LuaResult<usize> {
    let arg = l.get_arg(1);

    if let Some(arg_val) = arg {
        // Set new input file
        if let Some(filename) = arg_val.as_str() {
            // Open file for reading
            let lua_file = match LuaFile::open_read(filename) {
                Ok(f) => f,
                Err(e) => {
                    return Err(l.error(format!("cannot open file '{}': {}", filename, e)));
                }
            };
            let file_mt = create_file_metatable(l)?;
            let userdata = l.create_userdata(LuaUserdata::new(lua_file))?;

            if let Some(ud) = userdata.as_userdata_mut() {
                ud.set_metatable(file_mt);
            }

            l.vm_mut().gc.check_finalizer(&userdata);

            // Store in registry and update cache
            let registry = l.vm_mut().registry;
            let key = l.create_string("_IO_input")?;
            l.raw_set(&registry, key, userdata);
            l.vm_mut().io_default_input = Some(userdata);
        } else if arg_val.is_userdata() {
            // Verify it's a valid file handle
            if let Some(ud) = arg_val.as_userdata_mut() {
                let data = ud.get_data_mut();
                if data.downcast_ref::<LuaFile>().is_none() {
                    return Err(crate::stdlib::debug::arg_typeerror(l, 1, "FILE*", &arg_val));
                }
            }

            // Store in registry and update cache
            let registry = l.vm_mut().registry;
            let key = l.create_string("_IO_input")?;
            l.raw_set(&registry, key, arg_val);
            l.vm_mut().io_default_input = Some(arg_val);
        } else {
            return Err(crate::stdlib::debug::arg_typeerror(l, 1, "FILE*", &arg_val));
        }
    }

    // Return current input file
    let handle = get_default_input(l)?;
    l.push_value(handle)?;
    Ok(1)
}

/// io.output([file]) - Set or get default output file
fn io_output(l: &mut LuaState) -> LuaResult<usize> {
    let arg = l.get_arg(1);

    if let Some(arg_val) = arg {
        // Set new output file
        if let Some(filename) = arg_val.as_str() {
            // Create parent directories if they don't exist
            if let Some(parent) = std::path::Path::new(filename).parent()
                && !parent.as_os_str().is_empty()
            {
                let _ = std::fs::create_dir_all(parent);
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

            // Store in registry and update cache
            let registry = l.vm_mut().registry;
            let key = l.create_string("_IO_output")?;
            l.raw_set(&registry, key, userdata);
            l.vm_mut().io_default_output = Some(userdata);
        } else if arg_val.is_userdata() {
            // Verify it's a valid file handle
            if let Some(ud) = arg_val.as_userdata_mut() {
                let data = ud.get_data_mut();
                if data.downcast_ref::<LuaFile>().is_none() {
                    return Err(l.error("bad argument #1 to 'output' (file expected)".to_string()));
                }
            }

            // Store in registry and update cache
            let registry = l.vm_mut().registry;
            let key = l.create_string("_IO_output")?;
            l.raw_set(&registry, key, arg_val);
            l.vm_mut().io_default_output = Some(arg_val);
        } else {
            return Err(
                l.error("bad argument #1 to 'output' (string or file expected)".to_string())
            );
        }
    }

    // Return current output file
    let handle = get_default_output(l)?;
    l.push_value(handle)?;
    Ok(1)
}

/// io.type(obj) - Check if obj is a file handle
fn io_type(l: &mut LuaState) -> LuaResult<usize> {
    let obj = l.get_arg(1);

    if let Some(val) = obj
        && let Some(ud) = val.as_userdata_mut()
    {
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

            // Wrap in LuaFile (read+write mode for tmpfile)
            let lua_file = LuaFile::from_file_rw(file);

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

    let is_default_output = file_arg.is_none();
    let file_val = if let Some(file) = file_arg {
        file
    } else {
        // No file given - close default output
        get_default_output(l)?
    };

    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            // Cannot close already-closed files
            if lua_file.is_closed() {
                return Err(l.error("attempt to use a closed file".to_string()));
            }
            // Cannot close standard streams - return nil, msg
            if lua_file.is_std_stream() {
                l.push_value(LuaValue::nil())?;
                let msg = l.create_string("cannot close standard file")?;
                l.push_value(msg)?;
                return Ok(2);
            }
            match lua_file.close() {
                Ok(_) => {
                    // Invalidate cached handle if we closed the default output
                    if is_default_output {
                        l.vm_mut().io_default_output = None;
                    }
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
