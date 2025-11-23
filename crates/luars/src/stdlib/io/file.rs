// File userdata implementation
// Provides file handles for IO operations

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, Write};

use crate::LuaVM;
use crate::lib_registry::{get_arg, get_args, require_arg};
use crate::lua_value::{LuaValue, LuaValueKind, MultiValue};
use crate::lua_vm::{LuaError, LuaResult};

/// File handle wrapper
pub struct LuaFile {
    inner: FileInner,
}

enum FileInner {
    Read(BufReader<File>),
    Write(BufWriter<File>),
    ReadWrite(File),
    Closed,
}

impl LuaFile {
    pub fn open_read(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        Ok(LuaFile {
            inner: FileInner::Read(BufReader::new(file)),
        })
    }

    pub fn open_write(path: &str) -> io::Result<Self> {
        let file = File::create(path)?;
        Ok(LuaFile {
            inner: FileInner::Write(BufWriter::new(file)),
        })
    }

    pub fn open_append(path: &str) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .append(true)
            .open(path)?;
        Ok(LuaFile {
            inner: FileInner::Write(BufWriter::new(file)),
        })
    }

    pub fn open_readwrite(path: &str) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        Ok(LuaFile {
            inner: FileInner::ReadWrite(file),
        })
    }

    /// Read operations
    pub fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        match &mut self.inner {
            FileInner::Read(reader) => {
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    Ok(None)
                } else {
                    // Remove trailing newline if present
                    if line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    Ok(Some(line))
                }
            }
            FileInner::ReadWrite(file) => {
                let mut reader = BufReader::new(file);
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    Ok(None)
                } else {
                    if line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    Ok(Some(line))
                }
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "File not opened for reading",
            )),
        }
    }

    pub fn read_all(&mut self) -> io::Result<String> {
        let mut content = String::new();
        match &mut self.inner {
            FileInner::Read(reader) => {
                reader.read_to_string(&mut content)?;
                Ok(content)
            }
            FileInner::ReadWrite(file) => {
                file.read_to_string(&mut content)?;
                Ok(content)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "File not opened for reading",
            )),
        }
    }

    pub fn read_bytes(&mut self, n: usize) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0u8; n];
        match &mut self.inner {
            FileInner::Read(reader) => {
                let bytes_read = reader.read(&mut buffer)?;
                buffer.truncate(bytes_read);
                Ok(buffer)
            }
            FileInner::ReadWrite(file) => {
                let bytes_read = file.read(&mut buffer)?;
                buffer.truncate(bytes_read);
                Ok(buffer)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "File not opened for reading",
            )),
        }
    }

    /// Write operations
    pub fn write(&mut self, data: &str) -> io::Result<()> {
        match &mut self.inner {
            FileInner::Write(writer) => {
                writer.write_all(data.as_bytes())?;
                Ok(())
            }
            FileInner::ReadWrite(file) => {
                file.write_all(data.as_bytes())?;
                Ok(())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "File not opened for writing",
            )),
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            FileInner::Write(writer) => writer.flush(),
            FileInner::ReadWrite(file) => file.flush(),
            _ => Ok(()),
        }
    }

    pub fn close(&mut self) -> io::Result<()> {
        // Flush before closing
        self.flush()?;
        // Replace the inner with Closed to drop the file handles
        self.inner = FileInner::Closed;
        Ok(())
    }
}

/// Create file metatable with methods
pub fn create_file_metatable(vm: &mut LuaVM) -> LuaResult<LuaValue> {
    let mt = vm.create_table();

    // Create __index table with methods
    let index_table = vm.create_table();

    // file:read([format])
    let read_key = vm.create_string("read");
    vm.table_set_with_meta(index_table, read_key, LuaValue::cfunction(file_read))?;

    // file:write(...)
    let write_key = vm.create_string("write");
    vm.table_set_with_meta(index_table, write_key, LuaValue::cfunction(file_write))?;
    // file:flush()
    let flush_key = vm.create_string("flush");
    vm.table_set_with_meta(index_table, flush_key, LuaValue::cfunction(file_flush))?;
    // file:close()
    let close_key = vm.create_string("close");
    vm.table_set_with_meta(index_table, close_key, LuaValue::cfunction(file_close))?;

    // file:lines([formats])
    let lines_key = vm.create_string("lines");
    vm.table_set_with_meta(index_table, lines_key, LuaValue::cfunction(file_lines))?;

    // file:seek([whence [, offset]])
    let seek_key = vm.create_string("seek");
    vm.table_set_with_meta(index_table, seek_key, LuaValue::cfunction(file_seek))?;

    // file:setvbuf(mode [, size])
    let setvbuf_key = vm.create_string("setvbuf");
    vm.table_set_with_meta(index_table, setvbuf_key, LuaValue::cfunction(file_setvbuf))?;

    let index_key = vm.create_string("__index");
    vm.table_set_with_meta(mt, index_key, index_table)?;

    Ok(mt)
}

/// file:read([format])
fn file_read(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // For method calls from Lua, register 1 is self (file object)
    let file_val = require_arg(vm, 0, "file.read")?;

    // Extract LuaFile from userdata
    unsafe {
        if let Some(ud) = vm.get_userdata(&file_val) {
            let data = ud.borrow().get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
                // Get format (default "*l") - register 2 is first argument after self
                let format_str = if let Some(fmt) = get_arg(vm, 2) {
                    if let Some(s) = fmt.as_string() {
                        s.as_str().to_string()
                    } else {
                        "*l".to_string()
                    }
                } else {
                    "*l".to_string()
                };
                let format = format_str.as_str();

                let result = match format {
                    "*l" | "*L" => match lua_file.read_line() {
                        Ok(Some(line)) => vm.create_string(&line),
                        Ok(None) => LuaValue::nil(),
                        Err(e) => return Err(LuaError::RuntimeError(format!("read error: {}", e))),
                    },
                    "*a" => match lua_file.read_all() {
                        Ok(content) => vm.create_string(&content),
                        Err(e) => return Err(LuaError::RuntimeError(format!("read error: {}", e))),
                    },
                    _ => {
                        // Try to parse as number (byte count)
                        if let Ok(n) = format.parse::<usize>() {
                            match lua_file.read_bytes(n) {
                                Ok(bytes) => {
                                    let s = String::from_utf8_lossy(&bytes);
                                    vm.create_string(&s)
                                }
                                Err(e) => {
                                    return Err(LuaError::RuntimeError(format!(
                                        "read error: {}",
                                        e
                                    )));
                                }
                            }
                        } else {
                            return Err(LuaError::RuntimeError(format!(
                                "invalid format: {}",
                                format
                            )));
                        }
                    }
                };

                return Ok(MultiValue::single(result));
            }
        }
    }

    Err(LuaError::RuntimeError("expected file handle".to_string()))
}

/// file:write(...)
fn file_write(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // For method calls from Lua, register 1 is self (file object)
    let file_val = require_arg(vm, 0, "file.write")?;

    // Extract LuaFile from userdata
    unsafe {
        if let Some(ud) = vm.get_userdata(&file_val) {
            let data = ud.borrow().get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
                // Write all arguments (starting from register 2)
                let args = get_args(vm);
                for i in 1..args.len() {
                    let val = &args[i];
                    if val.is_nil() {
                        break;
                    }

                    let text = match val.kind() {
                        LuaValueKind::String => {
                            if let Some(s) = val.as_string() {
                                s.as_str().to_string()
                            } else {
                                return Err(LuaError::RuntimeError(
                                    "write expects strings or numbers".to_string(),
                                ));
                            }
                        }
                        LuaValueKind::Integer => {
                            if let Some(n) = val.as_integer() {
                                n.to_string()
                            } else {
                                return Err(LuaError::RuntimeError(
                                    "write expects strings or numbers".to_string(),
                                ));
                            }
                        }
                        LuaValueKind::Float => {
                            if let Some(n) = val.as_float() {
                                n.to_string()
                            } else {
                                return Err(LuaError::RuntimeError(
                                    "write expects strings or numbers".to_string(),
                                ));
                            }
                        }
                        _ => {
                            return Err(LuaError::RuntimeError(
                                "write expects strings or numbers".to_string(),
                            ));
                        }
                    };

                    if let Err(e) = lua_file.write(&text) {
                        return Err(LuaError::RuntimeError(format!("write error: {}", e)));
                    }
                }

                return Ok(MultiValue::single(file_val.clone()));
            }
        }
    }

    Err(LuaError::RuntimeError("expected file handle".to_string()))
}

/// file:flush()
fn file_flush(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // For method calls from Lua, register 1 is self (file object)
    let file_val = require_arg(vm, 0, "file.flush")?;

    // Extract LuaFile from userdata
    if let Some(ud) = vm.get_userdata(&file_val) {
        let data = ud.borrow().get_data();
        let mut data_ref = data.borrow_mut();
        if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
            if let Err(e) = lua_file.flush() {
                return Err(LuaError::RuntimeError(format!("flush error: {}", e)));
            }
            return Ok(MultiValue::single(LuaValue::boolean(true)));
        }
    }

    Err(LuaError::RuntimeError("expected file handle".to_string()))
}

/// file:close()
fn file_close(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // For method calls from Lua, register 1 is self (file object)
    let file_val = require_arg(vm, 0, "file.close")?;

    // Extract LuaFile from userdata
    if let Some(ud) = vm.get_userdata(&file_val) {
        let data = ud.borrow().get_data();
        let mut data_ref = data.borrow_mut();
        if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
            if let Err(e) = lua_file.close() {
                return Err(LuaError::RuntimeError(format!("close error: {}", e)));
            }
            return Ok(MultiValue::single(LuaValue::boolean(true)));
        }
    }

    Err(LuaError::RuntimeError("expected file handle".to_string()))
}

/// file:lines([formats]) - Returns an iterator for reading lines
fn file_lines(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Get file handle from self
    let file_val = require_arg(vm, 0, "file.lines")?;

    // For now, return a simple iterator that reads lines
    // Create state table with file handle
    let state_table = vm.create_table();
    let file_key = vm.create_string("file");
    let state_ref = vm
        .get_table(&state_table)
        .ok_or(LuaError::RuntimeError("Invalid state table".to_string()))?;
    state_ref.borrow_mut().raw_set(file_key, file_val.clone());

    Ok(MultiValue::multiple(vec![
        LuaValue::cfunction(file_lines_iterator),
        state_table,
        LuaValue::nil(),
    ]))
}

/// Iterator function for file:lines()
fn file_lines_iterator(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let state_val = require_arg(vm, 0, "iterator requires state")?;
    let file_key = vm.create_string("file");
    let state_ref_cell = vm
        .get_table(&state_val)
        .ok_or(LuaError::RuntimeError("Invalid state table".to_string()))?;
    let file_val = state_ref_cell
        .borrow()
        .raw_get(&file_key)
        .ok_or(LuaError::RuntimeError(
            "file not found in state".to_string(),
        ))?;

    // Read next line
    if let Some(ud) = vm.get_userdata(&file_val) {
        let data = ud.borrow().get_data();
        let mut data_ref = data.borrow_mut();
        if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
            match lua_file.read_line() {
                Ok(Some(line)) => {
                    return Ok(MultiValue::single(vm.create_string(&line)));
                }
                Ok(None) => return Ok(MultiValue::single(LuaValue::nil())),
                Err(e) => return Err(LuaError::RuntimeError(format!("read error: {}", e))),
            }
        }
    }

    Err(LuaError::RuntimeError("expected file handle".to_string()))
}

/// file:seek([whence [, offset]]) - Sets and gets the file position
fn file_seek(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let file_val = require_arg(vm, 0, "file.seek")?;

    let whence = get_arg(vm, 2)
        .and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) })
        .unwrap_or_else(|| "cur".to_string());

    let offset = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(0);

    if let Some(ud) = vm.get_userdata(&file_val) {
        let data = ud.borrow().get_data();
        let mut data_ref = data.borrow_mut();
        if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
            let seek_from = match whence.as_str() {
                "set" => std::io::SeekFrom::Start(offset.max(0) as u64),
                "cur" => std::io::SeekFrom::Current(offset),
                "end" => std::io::SeekFrom::End(offset),
                _ => {
                    return Err(LuaError::RuntimeError(format!(
                        "invalid whence: {}",
                        whence
                    )));
                }
            };

            let pos = match &mut lua_file.inner {
                FileInner::Read(reader) => reader.seek(seek_from),
                FileInner::Write(_) => {
                    return Err(LuaError::RuntimeError(
                        "cannot seek on write-only file".to_string(),
                    ));
                }
                FileInner::ReadWrite(file) => file.seek(seek_from),
                FileInner::Closed => {
                    return Err(LuaError::RuntimeError("file is closed".to_string()));
                }
            };

            match pos {
                Ok(position) => {
                    return Ok(MultiValue::single(LuaValue::integer(position as i64)));
                }
                Err(e) => return Err(LuaError::RuntimeError(format!("seek error: {}", e))),
            }
        }
    }

    Err(LuaError::RuntimeError("expected file handle".to_string()))
}

/// file:setvbuf(mode [, size]) - Sets the buffering mode
fn file_setvbuf(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let file_val = require_arg(vm, 0, "file.setvbuf")?;

    let _mode = get_arg(vm, 2)
        .and_then(|v| unsafe { v.as_string().map(|s| s.as_str().to_string()) })
        .unwrap_or_else(|| "full".to_string());

    let _size = get_arg(vm, 3).and_then(|v| v.as_integer());

    // Simplified implementation - just return success
    // In a full implementation, this would adjust buffering behavior
    Ok(MultiValue::single(file_val.clone()))
}
