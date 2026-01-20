// File userdata implementation
// Provides file handles for IO operations

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, Write};

use crate::lua_value::{LuaValue, LuaValueKind};
use crate::lua_vm::{LuaResult, LuaState};

/// File handle wrapper - supports files and standard streams
pub struct LuaFile {
    inner: FileInner,
}

enum FileInner {
    Read(BufReader<File>),
    Write(BufWriter<File>),
    ReadWrite(File),
    Stdin,
    Stdout,
    Stderr,
    Closed,
}

impl LuaFile {
    /// Create stdin handle
    pub fn stdin() -> Self {
        LuaFile {
            inner: FileInner::Stdin,
        }
    }

    /// Create stdout handle
    pub fn stdout() -> Self {
        LuaFile {
            inner: FileInner::Stdout,
        }
    }

    /// Create stderr handle
    pub fn stderr() -> Self {
        LuaFile {
            inner: FileInner::Stderr,
        }
    }

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
            .create(true)
            .open(path)?;
        Ok(LuaFile {
            inner: FileInner::Write(BufWriter::new(file)),
        })
    }

    pub fn open_readwrite(path: &str) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;
        Ok(LuaFile {
            inner: FileInner::ReadWrite(file),
        })
    }

    /// Create from existing File (for tmpfile)
    pub fn from_file(file: File) -> Self {
        LuaFile {
            inner: FileInner::ReadWrite(file),
        }
    }

    /// Check if file is closed
    pub fn is_closed(&self) -> bool {
        matches!(self.inner, FileInner::Closed)
    }

    /// Check if this is a standard stream (stdin/stdout/stderr)
    pub fn is_std_stream(&self) -> bool {
        matches!(
            self.inner,
            FileInner::Stdin | FileInner::Stdout | FileInner::Stderr
        )
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
            FileInner::Stdin => {
                let stdin = io::stdin();
                let n = stdin.lock().read_line(&mut line)?;
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
            FileInner::Stdin => {
                io::stdin().lock().read_to_string(&mut content)?;
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
            FileInner::Stdin => {
                let bytes_read = io::stdin().lock().read(&mut buffer)?;
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
            FileInner::Stdout => {
                io::stdout().write_all(data.as_bytes())?;
                Ok(())
            }
            FileInner::Stderr => {
                io::stderr().write_all(data.as_bytes())?;
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
            FileInner::Stdout => io::stdout().flush(),
            FileInner::Stderr => io::stderr().flush(),
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
pub fn create_file_metatable(l: &mut LuaState) -> LuaResult<LuaValue> {
    let mt = l.create_table(0, 1);

    // Create __index table with methods
    let index_table = l.create_table(0, 7);

    // file:read([format])
    let read_key = l.create_string("read");
    l.table_set(&index_table, read_key, LuaValue::cfunction(file_read));

    // file:write(...)
    let write_key = l.create_string("write");
    l.table_set(&index_table, write_key, LuaValue::cfunction(file_write));

    // file:flush()
    let flush_key = l.create_string("flush");
    l.table_set(&index_table, flush_key, LuaValue::cfunction(file_flush));

    // file:close()
    let close_key = l.create_string("close");
    l.table_set(&index_table, close_key, LuaValue::cfunction(file_close));

    // file:lines([formats])
    let lines_key = l.create_string("lines");
    l.table_set(&index_table, lines_key, LuaValue::cfunction(file_lines));

    // file:seek([whence [, offset]])
    let seek_key = l.create_string("seek");
    l.table_set(&index_table, seek_key, LuaValue::cfunction(file_seek));

    // file:setvbuf(mode [, size])
    let setvbuf_key = l.create_string("setvbuf");
    l.table_set(&index_table, setvbuf_key, LuaValue::cfunction(file_setvbuf));

    let index_key = l.create_string("__index");
    l.table_set(&mt, index_key, index_table);

    Ok(mt)
}

/// file:read([format])
fn file_read(l: &mut LuaState) -> LuaResult<usize> {
    // For method calls from Lua, arg 1 is self (file object)
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:read requires file handle".to_string()))?;

    // Extract LuaFile from userdata using direct pointer access
    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            // Get format (default "*l")
            let format_arg = l.get_arg(2);

            // Check if format is a number (byte count)
            if let Some(ref fmt) = format_arg {
                if let Some(n) = fmt.as_integer() {
                    // Read n bytes
                    match lua_file.read_bytes(n as usize) {
                        Ok(bytes) => {
                            if bytes.is_empty() {
                                l.push_value(LuaValue::nil())?;
                                return Ok(1);
                            }
                            let s = String::from_utf8_lossy(&bytes);
                            let str_val = l.create_string(&s);
                            l.push_value(str_val)?;
                            return Ok(1);
                        }
                        Err(e) => {
                            return Err(l.error(format!("read error: {}", e)));
                        }
                    }
                }
            }

            // Otherwise treat as format string
            let format_str = format_arg
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "*l".to_string());
            let format = format_str.as_str();

            let result: LuaValue = match format {
                "*l" | "*L" => {
                    let res = lua_file.read_line();
                    match res {
                        Ok(Some(line)) => l.create_string(&line),
                        Ok(None) => LuaValue::nil(),
                        Err(e) => return Err(l.error(format!("read error: {}", e))),
                    }
                }
                "*a" => {
                    let res = lua_file.read_all();
                    match res {
                        Ok(content) => l.create_string(&content),
                        Err(e) => return Err(l.error(format!("read error: {}", e))),
                    }
                }
                "*n" => {
                    // Read a number
                    match lua_file.read_line() {
                        Ok(Some(line)) => {
                            let trimmed = line.trim();
                            if let Ok(n) = trimmed.parse::<i64>() {
                                LuaValue::integer(n)
                            } else if let Ok(n) = trimmed.parse::<f64>() {
                                LuaValue::float(n)
                            } else {
                                LuaValue::nil()
                            }
                        }
                        Ok(None) => LuaValue::nil(),
                        Err(e) => return Err(l.error(format!("read error: {}", e))),
                    }
                }
                _ => {
                    // Try to parse as number (byte count) from string like "10"
                    if let Ok(n) = format.parse::<usize>() {
                        match lua_file.read_bytes(n) {
                            Ok(bytes) => {
                                if bytes.is_empty() {
                                    LuaValue::nil()
                                } else {
                                    let s = String::from_utf8_lossy(&bytes);
                                    l.create_string(&s)
                                }
                            }
                            Err(e) => {
                                return Err(l.error(format!("read error: {}", e)));
                            }
                        }
                    } else {
                        return Err(l.error(format!("invalid format: {}", format)));
                    }
                }
            };

            l.push_value(result)?;
            return Ok(1);
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// file:write(...)
fn file_write(l: &mut LuaState) -> LuaResult<usize> {
    // For method calls from Lua, arg 1 is self (file object)
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:write requires file handle".to_string()))?;

    // Extract LuaFile from userdata using direct pointer access
    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            // Write all arguments (starting from arg 2)
            let mut i = 2;
            loop {
                let val = match l.get_arg(i) {
                    Some(v) => v,
                    None => break,
                };

                match val.kind() {
                    LuaValueKind::String => {
                        if let Some(s) = val.as_str() {
                            if let Err(e) = lua_file.write(&s) {
                                return Err(l.error(format!("write error: {}", e)));
                            }
                        } else {
                            return Err(l.error("write expects strings or numbers".to_string()));
                        }
                    }
                    LuaValueKind::Integer => {
                        if let Some(n) = val.as_integer() {
                            if let Err(e) = lua_file.write(&n.to_string()) {
                                return Err(l.error(format!("write error: {}", e)));
                            }
                        } else {
                            return Err(l.error("write expects strings or numbers".to_string()));
                        }
                    }
                    LuaValueKind::Float => {
                        if let Some(n) = val.as_float() {
                            if let Err(e) = lua_file.write(&n.to_string()) {
                                return Err(l.error(format!("write error: {}", e)));
                            }
                        } else {
                            return Err(l.error("write expects strings or numbers".to_string()));
                        }
                    }
                    _ => {
                        return Err(l.error("write expects strings or numbers".to_string()));
                    }
                };

                i += 1;
            }

            l.push_value(file_val)?;
            return Ok(1);
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// file:flush()
fn file_flush(l: &mut LuaState) -> LuaResult<usize> {
    // For method calls from Lua, arg 1 is self (file object)
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:flush requires file handle".to_string()))?;

    // Extract LuaFile from userdata using direct pointer access
    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            if let Err(e) = lua_file.flush() {
                return Err(l.error(format!("flush error: {}", e)));
            }
            l.push_value(LuaValue::boolean(true))?;
            return Ok(1);
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// file:close()
fn file_close(l: &mut LuaState) -> LuaResult<usize> {
    // For method calls from Lua, arg 1 is self (file object)
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:close requires file handle".to_string()))?;

    // Extract LuaFile from userdata using direct pointer access
    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            if let Err(e) = lua_file.close() {
                return Err(l.error(format!("close error: {}", e)));
            }
            l.push_value(LuaValue::boolean(true))?;
            return Ok(1);
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// file:lines([formats]) - Returns an iterator for reading lines
fn file_lines(l: &mut LuaState) -> LuaResult<usize> {
    // Get file handle from self
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:lines requires file handle".to_string()))?;

    // For now, return a simple iterator that reads lines
    // Create state table with file handle
    let state_table = l.create_table(0, 1);
    let file_key = l.create_string("file");
    l.table_set(&state_table, file_key, file_val);

    l.push_value(LuaValue::cfunction(file_lines_iterator))?;
    l.push_value(state_table)?;
    l.push_value(LuaValue::nil())?;
    Ok(3)
}

/// Iterator function for file:lines()
fn file_lines_iterator(l: &mut LuaState) -> LuaResult<usize> {
    let state_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("iterator requires state".to_string()))?;
    let file_key = l.create_string("file");
    let file_val = l
        .table_get(&state_val, &file_key)
        .ok_or_else(|| l.error("file not found in state".to_string()))?;

    // Read next line
    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
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

/// file:seek([whence [, offset]]) - Sets and gets the file position
fn file_seek(l: &mut LuaState) -> LuaResult<usize> {
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:seek requires file handle".to_string()))?;

    let whence = l
        .get_arg(2)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "cur".to_string());

    let offset = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(0);

    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            let seek_from = match whence.as_str() {
                "set" => std::io::SeekFrom::Start(offset.max(0) as u64),
                "cur" => std::io::SeekFrom::Current(offset),
                "end" => std::io::SeekFrom::End(offset),
                _ => {
                    return Err(l.error(format!("invalid whence: {}", whence)));
                }
            };

            let pos = match &mut lua_file.inner {
                FileInner::Read(reader) => reader.seek(seek_from),
                FileInner::Write(_) => {
                    return Err(l.error("cannot seek on write-only file".to_string()));
                }
                FileInner::ReadWrite(file) => file.seek(seek_from),
                FileInner::Closed => {
                    return Err(l.error("file is closed".to_string()));
                }
                FileInner::Stdin | FileInner::Stdout | FileInner::Stderr => {
                    return Err(l.error("cannot seek on standard stream".to_string()));
                }
            };

            match pos {
                Ok(position) => {
                    l.push_value(LuaValue::integer(position as i64))?;
                    return Ok(1);
                }
                Err(e) => return Err(l.error(format!("seek error: {}", e))),
            }
        }
    }

    Err(l.error("expected file handle".to_string()))
}

/// file:setvbuf(mode [, size]) - Sets the buffering mode
fn file_setvbuf(l: &mut LuaState) -> LuaResult<usize> {
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:setvbuf requires file handle".to_string()))?;

    let _mode = l
        .get_arg(2)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "full".to_string());

    let _size = l.get_arg(3).and_then(|v| v.as_integer());

    // Simplified implementation - just return success
    // In a full implementation, this would adjust buffering behavior
    l.push_value(file_val)?;
    Ok(1)
}
