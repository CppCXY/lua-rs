// File userdata implementation
// Provides file handles for IO operations

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::rc::Rc;

use crate::LuaVM;
use crate::lua_value::{LuaTable, LuaValue, LuaValueKind, MultiValue};

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
pub fn create_file_metatable(vm: &mut LuaVM) -> Rc<RefCell<LuaTable>> {
    let mt = Rc::new(RefCell::new(LuaTable::new()));

    // Create __index table with methods
    let index_table = Rc::new(RefCell::new(LuaTable::new()));

    // file:read([format])
    index_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("read".to_string())),
        LuaValue::cfunction(file_read),
    );

    // file:write(...)
    index_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("write".to_string())),
        LuaValue::cfunction(file_write),
    );

    // file:flush()
    index_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("flush".to_string())),
        LuaValue::cfunction(file_flush),
    );

    // file:close()
    index_table.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("close".to_string())),
        LuaValue::cfunction(file_close),
    );

    // Set __index to the index table
    mt.borrow_mut().raw_set(
        LuaValue::from_string_rc(vm.create_string("__index".to_string())),
        LuaValue::from_table_rc(index_table),
    );

    mt
}

/// file:read([format])
fn file_read(vm: &mut LuaVM) -> Result<MultiValue, String> {
    // For method calls from Lua, register 0 is the function, register 1 is self
    let frame = vm.frames.last().unwrap();
    let file_val = if frame.registers.len() > 1 {
        &frame.registers[1]
    } else {
        return Err("file:read requires self parameter".to_string());
    };

    // Extract LuaFile from userdata
    unsafe {
        if let Some(ud) = file_val.as_userdata() {
            let data = ud.get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
                // Get format (default "*l") - register 2 is first argument after self
                let format_str = if frame.registers.len() > 2 {
                    if let Some(s) = frame.registers[2].as_string() {
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
                        Ok(Some(line)) => LuaValue::from_string_rc(vm.create_string(line)),
                        Ok(None) => LuaValue::nil(),
                        Err(e) => return Err(format!("read error: {}", e)),
                    },
                    "*a" => match lua_file.read_all() {
                        Ok(content) => LuaValue::from_string_rc(vm.create_string(content)),
                        Err(e) => return Err(format!("read error: {}", e)),
                    },
                    _ => {
                        // Try to parse as number (byte count)
                        if let Ok(n) = format.parse::<usize>() {
                            match lua_file.read_bytes(n) {
                                Ok(bytes) => {
                                    let s = String::from_utf8_lossy(&bytes).to_string();
                                    LuaValue::from_string_rc(vm.create_string(s))
                                }
                                Err(e) => return Err(format!("read error: {}", e)),
                            }
                        } else {
                            return Err(format!("invalid format: {}", format));
                        }
                    }
                };

                return Ok(MultiValue::single(result));
            }
        }
    }

    Err("expected file handle".to_string())
}

/// file:write(...)
fn file_write(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let frame = vm.frames.last().unwrap();

    // For method calls from Lua, register 1 is self (file object)
    let file_val = if frame.registers.len() > 1 {
        &frame.registers[1]
    } else {
        return Err("file:write requires self parameter".to_string());
    };

    // Extract LuaFile from userdata
    unsafe {
        if let Some(ud) = file_val.as_userdata() {
            let data = ud.get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
                // Write all arguments (starting from register 2)
                for i in 2..frame.registers.len() {
                    let val = &frame.registers[i];
                    if val.is_nil() {
                        break;
                    }

                    let text = match val.kind() {
                        LuaValueKind::String => {
                            if let Some(s) = val.as_string() {
                                s.as_str().to_string()
                            } else {
                                return Err("write expects strings or numbers".to_string());
                            }
                        }
                        LuaValueKind::Integer => {
                            if let Some(n) = val.as_integer() {
                                n.to_string()
                            } else {
                                return Err("write expects strings or numbers".to_string());
                            }
                        }
                        LuaValueKind::Float => {
                            if let Some(n) = val.as_float() {
                                n.to_string()
                            } else {
                                return Err("write expects strings or numbers".to_string());
                            }
                        }
                        _ => return Err("write expects strings or numbers".to_string()),
                    };

                    if let Err(e) = lua_file.write(&text) {
                        return Err(format!("write error: {}", e));
                    }
                }

                return Ok(MultiValue::single(file_val.clone()));
            }
        }
    }

    Err("expected file handle".to_string())
}

/// file:flush()
fn file_flush(vm: &mut LuaVM) -> Result<MultiValue, String> {
    // For method calls from Lua, register 0 is the function, register 1 is self
    let frame = vm.frames.last().unwrap();
    let file_val = if frame.registers.len() > 1 {
        &frame.registers[1]
    } else {
        return Err("file:flush requires self parameter".to_string());
    };

    // Extract LuaFile from userdata
    unsafe {
        if let Some(ud) = file_val.as_userdata() {
            let data = ud.get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
                if let Err(e) = lua_file.flush() {
                    return Err(format!("flush error: {}", e));
                }
                return Ok(MultiValue::single(LuaValue::boolean(true)));
            }
        }
    }

    Err("expected file handle".to_string())
}

/// file:close()
fn file_close(vm: &mut LuaVM) -> Result<MultiValue, String> {
    // For method calls from Lua, register 0 is the function, register 1 is self
    let frame = vm.frames.last().unwrap();
    let file_val = if frame.registers.len() > 1 {
        &frame.registers[1]
    } else {
        return Err("file:close requires self parameter".to_string());
    };

    // Extract LuaFile from userdata
    unsafe {
        if let Some(ud) = file_val.as_userdata() {
            let data = ud.get_data();
            let mut data_ref = data.borrow_mut();
            if let Some(lua_file) = data_ref.downcast_mut::<LuaFile>() {
                if let Err(e) = lua_file.close() {
                    return Err(format!("close error: {}", e));
                }
                return Ok(MultiValue::single(LuaValue::boolean(true)));
            }
        }
    }

    Err("expected file handle".to_string())
}
