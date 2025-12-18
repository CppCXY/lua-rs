// IO library implementation
// Implements: close, flush, input, lines, open, output, read, write, type

use crate::lib_registry::{LibraryEntry, LibraryModule, get_arg, get_args, require_arg};
use crate::lua_value::{LuaUserdata, LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};
use std::io::{self, BufRead, Write};

mod file;
pub use file::{LuaFile, create_file_metatable};

pub fn create_io_lib() -> LibraryModule {
    let mut module = LibraryModule::new("io");

    // Functions
    module
        .entries
        .push(("write", LibraryEntry::Function(io_write)));
    module
        .entries
        .push(("read", LibraryEntry::Function(io_read)));
    module
        .entries
        .push(("flush", LibraryEntry::Function(io_flush)));
    module
        .entries
        .push(("open", LibraryEntry::Function(io_open)));
    module
        .entries
        .push(("lines", LibraryEntry::Function(io_lines)));
    module
        .entries
        .push(("input", LibraryEntry::Function(io_input)));
    module
        .entries
        .push(("output", LibraryEntry::Function(io_output)));
    module
        .entries
        .push(("type", LibraryEntry::Function(io_type)));
    module
        .entries
        .push(("tmpfile", LibraryEntry::Function(io_tmpfile)));
    module
        .entries
        .push(("close", LibraryEntry::Function(io_close)));
    module
        .entries
        .push(("popen", LibraryEntry::Function(io_popen)));

    // Standard streams
    module
        .entries
        .push(("stdin", LibraryEntry::Value(create_stdin)));
    module
        .entries
        .push(("stdout", LibraryEntry::Value(create_stdout)));
    module
        .entries
        .push(("stderr", LibraryEntry::Value(create_stderr)));

    module
}

/// Create stdin file handle
fn create_stdin(vm: &mut LuaVM) -> LuaValue {
    let file = LuaFile::stdin();
    let file_mt = create_file_metatable(vm).unwrap_or(LuaValue::nil());
    let userdata = vm.create_userdata(LuaUserdata::new(file));
    if let Some(ud_id) = userdata.as_userdata_id() {
        if let Some(ud) = vm.object_pool.get_userdata_mut(ud_id) {
            ud.set_metatable(file_mt);
        }
    }
    userdata
}

/// Create stdout file handle
fn create_stdout(vm: &mut LuaVM) -> LuaValue {
    let file = LuaFile::stdout();
    let file_mt = create_file_metatable(vm).unwrap_or(LuaValue::nil());
    let userdata = vm.create_userdata(LuaUserdata::new(file));
    if let Some(ud_id) = userdata.as_userdata_id() {
        if let Some(ud) = vm.object_pool.get_userdata_mut(ud_id) {
            ud.set_metatable(file_mt);
        }
    }
    userdata
}

/// Create stderr file handle
fn create_stderr(vm: &mut LuaVM) -> LuaValue {
    let file = LuaFile::stderr();
    let file_mt = create_file_metatable(vm).unwrap_or(LuaValue::nil());
    let userdata = vm.create_userdata(LuaUserdata::new(file));
    if let Some(ud_id) = userdata.as_userdata_id() {
        if let Some(ud) = vm.object_pool.get_userdata_mut(ud_id) {
            ud.set_metatable(file_mt);
        }
    }
    userdata
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
    let filename = get_arg(vm, 1);

    if let Some(filename_val) = filename {
        // io.lines(filename) - open file and return iterator
        let filename_str = match vm.get_string(&filename_val) {
            Some(s) => s.as_str().to_string(),
            None => return Err(vm.error("bad argument #1 to 'lines' (string expected)")),
        };

        // Open the file
        match LuaFile::open_read(&filename_str) {
            Ok(file) => {
                // Create file metatable
                let file_mt = create_file_metatable(vm)?;

                // Create userdata
                use crate::lua_value::LuaUserdata;
                let userdata = vm.create_userdata(LuaUserdata::new(file));

                // Set metatable
                if let Some(ud_id) = userdata.as_userdata_id() {
                    if let Some(ud) = vm.object_pool.get_userdata_mut(ud_id) {
                        ud.set_metatable(file_mt);
                    }
                }

                // Create state table with file handle
                let state_table = vm.create_table(0, 1);
                let file_key = vm.create_string("file");
                vm.table_set_raw(&state_table, file_key, userdata);

                Ok(MultiValue::multiple(vec![
                    LuaValue::cfunction(io_lines_iterator),
                    state_table,
                    LuaValue::nil(),
                ]))
            }
            Err(e) => Err(vm.error(format!("cannot open file '{}': {}", filename_str, e))),
        }
    } else {
        // io.lines() - read from stdin
        Err(vm.error("io.lines() without filename not yet implemented"))
    }
}

/// Iterator function for io.lines()
fn io_lines_iterator(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let state_val = require_arg(vm, 1, "iterator requires state")?;
    let file_key = vm.create_string("file");
    let file_val = vm.table_get_raw(&state_val, &file_key);
    if file_val.is_nil() {
        return Err(vm.error("file not found in state".to_string()));
    }

    // Read next line
    if let Some(ud) = vm.get_userdata(&file_val) {
        let data = ud.get_data();
        let mut data_ref = data.borrow_mut();
        if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
            match lua_file.read_line() {
                Ok(Some(line)) => {
                    return Ok(MultiValue::single(vm.create_string(&line)));
                }
                Ok(None) => return Ok(MultiValue::single(LuaValue::nil())),
                Err(e) => return Err(vm.error(format!("read error: {}", e))),
            }
        }
    }

    Err(vm.error("expected file handle".to_string()))
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

/// io.type(obj) - Check if obj is a file handle
fn io_type(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let obj = get_arg(vm, 1);

    if let Some(val) = obj {
        if let Some(ud) = vm.get_userdata(&val) {
            let data = ud.get_data();
            let data_ref = data.borrow();
            if data_ref.downcast_ref::<LuaFile>().is_some() {
                // Check if file is closed
                drop(data_ref);
                let data_ref = data.borrow();
                if let Some(lua_file) = data_ref.downcast_ref::<LuaFile>() {
                    if lua_file.is_closed() {
                        return Ok(MultiValue::single(vm.create_string("closed file")));
                    } else {
                        return Ok(MultiValue::single(vm.create_string("file")));
                    }
                }
            }
        }
    }

    Ok(MultiValue::single(LuaValue::nil()))
}

/// io.tmpfile() - Create a temporary file
fn io_tmpfile(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Create a temporary file
    match tempfile::tempfile() {
        Ok(file) => {
            // Wrap in LuaFile (need to add support for this)
            let lua_file = LuaFile::from_file(file);

            // Create file metatable
            let file_mt = create_file_metatable(vm)?;

            // Create userdata
            use crate::lua_value::LuaUserdata;
            let userdata = vm.create_userdata(LuaUserdata::new(lua_file));

            // Set metatable
            if let Some(ud_id) = userdata.as_userdata_id() {
                if let Some(ud) = vm.object_pool.get_userdata_mut(ud_id) {
                    ud.set_metatable(file_mt);
                }
            }

            Ok(MultiValue::single(userdata))
        }
        Err(e) => Ok(MultiValue::multiple(vec![
            LuaValue::nil(),
            vm.create_string(&e.to_string()),
        ])),
    }
}

/// io.close([file]) - Close a file
fn io_close(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let file_arg = get_arg(vm, 1);

    if let Some(file_val) = file_arg {
        if let Some(ud) = vm.get_userdata(&file_val) {
            let data = ud.get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
                // Don't actually close standard streams
                if lua_file.is_std_stream() {
                    return Ok(MultiValue::single(LuaValue::boolean(true)));
                }
                match lua_file.close() {
                    Ok(_) => return Ok(MultiValue::single(LuaValue::boolean(true))),
                    Err(e) => return Err(vm.error(format!("close error: {}", e))),
                }
            }
        }
        return Err(vm.error("expected file handle"));
    }

    // No argument - close default output
    Ok(MultiValue::single(LuaValue::boolean(true)))
}

/// io.popen(prog [, mode]) - Execute program and return file handle
fn io_popen(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // io.popen is platform-specific and potentially dangerous
    // Return nil + error message to indicate it's not available
    Ok(MultiValue::multiple(vec![
        LuaValue::nil(),
        vm.create_string("io.popen not available in this environment"),
    ]))
}
