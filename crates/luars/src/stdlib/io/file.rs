// File userdata implementation
// Provides file handles for IO operations

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, Write};

use crate::lua_value::{LuaValue, LuaValueKind};
use crate::lua_value::userdata_trait::UserDataTrait;
use crate::lua_vm::{LuaResult, LuaState};

pub enum ReadNumberResult {
    Integer(i64),
    Float(f64),
}

/// Parse a Lua integer literal (decimal or hex, with wrapping for hex)
fn parse_lua_integer(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        let hex = &s[2..];
        // Parse as u64 to allow wrapping (e.g., 0x8000000000000001 -> negative i64)
        u64::from_str_radix(hex, 16).ok().map(|n| n as i64)
    } else if s.starts_with("-0x") || s.starts_with("-0X") {
        let hex = &s[3..];
        u64::from_str_radix(hex, 16).ok().map(|n| (n as i64).wrapping_neg())
    } else {
        s.parse::<i64>().ok()
    }
}

/// Parse a Lua float literal (including hex floats like 0xABCp-3)
fn parse_lua_float(s: &str) -> Option<f64> {
    let s = s.trim();
    // Standard float
    if let Ok(n) = s.parse::<f64>() {
        return Some(n);
    }
    // Hex float: 0xHHH[.HHH][pEXP]
    let (neg, rest) = if s.starts_with('-') {
        (true, &s[1..])
    } else if s.starts_with('+') {
        (false, &s[1..])
    } else {
        (false, s)
    };
    if rest.starts_with("0x") || rest.starts_with("0X") {
        let hex = &rest[2..];
        // Split at 'p' or 'P'
        let (mantissa_str, exp_str) = if let Some(p) = hex.find(|c: char| c == 'p' || c == 'P') {
            (&hex[..p], Some(&hex[p+1..]))
        } else {
            (hex, None)
        };
        // Parse mantissa (integer part . fractional part)
        let (int_str, frac_str) = if let Some(dot) = mantissa_str.find('.') {
            (&mantissa_str[..dot], Some(&mantissa_str[dot+1..]))
        } else {
            (mantissa_str, None)
        };
        let int_val = u64::from_str_radix(int_str, 16).ok()? as f64;
        let frac_val = if let Some(frac) = frac_str {
            if frac.is_empty() {
                0.0
            } else {
                let frac_num = u64::from_str_radix(frac, 16).ok()? as f64;
                frac_num / 16f64.powi(frac.len() as i32)
            }
        } else {
            0.0
        };
        let mut result = int_val + frac_val;
        if let Some(exp) = exp_str {
            let exp_val: i32 = exp.parse().ok()?;
            result *= 2f64.powi(exp_val);
        }
        if neg { result = -result; }
        return Some(result);
    }
    None
}

/// File handle wrapper - supports files and standard streams
pub struct LuaFile {
    inner: FileInner,
}

enum FileInner {
    Read(BufReader<File>),
    Write(BufWriter<File>),
    ReadWrite(BufReader<File>),
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
            inner: FileInner::ReadWrite(BufReader::new(file)),
        })
    }

    /// Open for reading and appending (a+ mode)
    pub fn open_append_read(path: &str) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .append(true)
            .create(true)
            .open(path)?;
        Ok(LuaFile {
            inner: FileInner::ReadWrite(BufReader::new(file)),
        })
    }

    /// Open for reading and writing, truncating (w+ mode)
    pub fn open_write_read(path: &str) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(LuaFile {
            inner: FileInner::ReadWrite(BufReader::new(file)),
        })
    }

    /// Create from existing File (for tmpfile)
    pub fn from_file(file: File) -> Self {
        LuaFile {
            inner: FileInner::ReadWrite(BufReader::new(file)),
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
            FileInner::ReadWrite(reader) => {
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

    /// Read a line, keeping the trailing newline
    pub fn read_line_with_newline(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        match &mut self.inner {
            FileInner::Read(reader) => {
                let n = reader.read_line(&mut line)?;
                if n == 0 { Ok(None) } else { Ok(Some(line)) }
            }
            FileInner::ReadWrite(reader) => {
                let n = reader.read_line(&mut line)?;
                if n == 0 { Ok(None) } else { Ok(Some(line)) }
            }
            FileInner::Stdin => {
                let stdin = io::stdin();
                let n = stdin.lock().read_line(&mut line)?;
                if n == 0 { Ok(None) } else { Ok(Some(line)) }
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "File not opened for reading",
            )),
        }
    }

    /// Read a number from the file (like C Lua's read_number).
    /// Follows C Lua's state machine: skip whitespace, optional sign,
    /// optional 0x/0X prefix, digits, optional decimal point + digits,
    /// optional exponent (e/E or p/P) with optional sign + digits.
    pub fn read_number(&mut self) -> io::Result<Option<ReadNumberResult>> {
        use std::io::Read;
        
        let mut buf = Vec::new();
        let mut byte_buf = [0u8; 1];
        let mut last_byte: Option<u8> = None; // for ungetc
        const L_MAXLENNUM: usize = 200; // max length of a numeral (C Lua's L_MAXLENNUM)
        
        // Read one byte (with pushback support)
        let read_byte_inner = |inner: &mut FileInner, byte_buf: &mut [u8; 1]| -> io::Result<Option<u8>> {
            match inner {
                FileInner::Read(reader) => {
                    let n = reader.read(&mut byte_buf[..])?;
                    if n == 0 { Ok(None) } else { Ok(Some(byte_buf[0])) }
                }
                FileInner::ReadWrite(reader) => {
                    let n = reader.read(&mut byte_buf[..])?;
                    if n == 0 { Ok(None) } else { Ok(Some(byte_buf[0])) }
                }
                FileInner::Stdin => {
                    let n = io::stdin().lock().read(&mut byte_buf[..])?;
                    if n == 0 { Ok(None) } else { Ok(Some(byte_buf[0])) }
                }
                _ => Err(io::Error::new(io::ErrorKind::Other, "not readable")),
            }
        };
        
        let ungetc = |inner: &mut FileInner| {
            match inner {
                FileInner::Read(reader) => { let _ = reader.seek(std::io::SeekFrom::Current(-1)); }
                FileInner::ReadWrite(reader) => { let _ = reader.seek(std::io::SeekFrom::Current(-1)); }
                _ => {}
            }
        };
        
        // Macro-like helpers
        // test2: if current char matches c1 or c2, add to buf and read next
        macro_rules! getc {
            ($self:expr) => {{
                if let Some(b) = last_byte.take() {
                    Some(b)
                } else {
                    read_byte_inner(&mut $self.inner, &mut byte_buf)?
                }
            }};
        }
        
        // Skip whitespace
        let mut c = loop {
            match getc!(self) {
                None => return Ok(None),
                Some(b) if (b as char).is_ascii_whitespace() => continue,
                Some(b) => break b,
            }
        };
        
        // Optional sign
        if c == b'+' || c == b'-' {
            buf.push(c);
            match getc!(self) {
                None => { /* end of file after sign */ return Ok(None); }
                Some(b) => c = b,
            }
        }
        
        // Check for hex prefix: 0x or 0X
        let mut digit_count: usize = 0;
        let hex = if c == b'0' {
            buf.push(c);
            digit_count += 1; // "0" counts as a digit
            match getc!(self) {
                None => {
                    // Just "0" — parse it
                    return Ok(Some(ReadNumberResult::Integer(0)));
                }
                Some(b) if b == b'x' || b == b'X' => {
                    buf.push(b);
                    digit_count = 0; // reset — need hex digits after prefix
                    match getc!(self) {
                        None => { c = 0; }
                        Some(b2) => c = b2,
                    }
                    true
                }
                Some(b) => {
                    c = b;
                    false
                }
            }
        } else {
            false
        };
        
        // Read digits (integral part)
        loop {
            let is_digit = if hex {
                (c as char).is_ascii_hexdigit()
            } else {
                (c as char).is_ascii_digit()
            };
            if !is_digit || buf.len() >= L_MAXLENNUM { break; }
            buf.push(c);
            digit_count += 1;
            match getc!(self) {
                None => { c = 0; break; }
                Some(b) => c = b,
            }
        }
        
        // Optional decimal point
        if c == b'.' {
            buf.push(c);
            match getc!(self) {
                None => { c = 0; }
                Some(b) => c = b,
            }
            // Fractional digits
            loop {
                let is_digit = if hex {
                    (c as char).is_ascii_hexdigit()
                } else {
                    (c as char).is_ascii_digit()
                };
                if !is_digit || buf.len() >= L_MAXLENNUM { break; }
                buf.push(c);
                digit_count += 1;
                match getc!(self) {
                    None => { c = 0; break; }
                    Some(b) => c = b,
                }
            }
        }
        
        // Optional exponent
        if digit_count > 0 {
            let exp_char = if hex { b'p' } else { b'e' };
            let exp_char_upper = if hex { b'P' } else { b'E' };
            if c == exp_char || c == exp_char_upper {
                buf.push(c);
                match getc!(self) {
                    None => { c = 0; }
                    Some(b) => c = b,
                }
                // Optional exponent sign
                if c == b'+' || c == b'-' {
                    buf.push(c);
                    match getc!(self) {
                        None => { c = 0; }
                        Some(b) => c = b,
                    }
                }
                // Exponent digits (always decimal)
                loop {
                    if !(c as char).is_ascii_digit() || buf.len() >= L_MAXLENNUM { break; }
                    buf.push(c);
                    match getc!(self) {
                        None => { c = 0; break; }
                        Some(b) => c = b,
                    }
                }
            }
        }
        
        // Unread the look-ahead character
        if c != 0 {
            ungetc(&mut self.inner);
        }
        
        if buf.is_empty() || digit_count == 0 {
            return Ok(None);
        }

        // If the buffer was truncated due to L_MAXLENNUM, fail
        if buf.len() >= L_MAXLENNUM {
            return Ok(None);
        }

        let s = String::from_utf8(buf).unwrap_or_default();
        let trimmed = s.trim();
        
        // Try integer first
        if let Some(n) = parse_lua_integer(trimmed) {
            return Ok(Some(ReadNumberResult::Integer(n)));
        }
        // Then float
        if let Some(n) = parse_lua_float(trimmed) {
            return Ok(Some(ReadNumberResult::Float(n)));
        }
        
        Ok(None)
    }

    pub fn read_all(&mut self) -> io::Result<Vec<u8>> {
        let mut content = Vec::new();
        match &mut self.inner {
            FileInner::Read(reader) => {
                reader.read_to_end(&mut content)?;
                Ok(content)
            }
            FileInner::ReadWrite(reader) => {
                reader.read_to_end(&mut content)?;
                Ok(content)
            }
            FileInner::Stdin => {
                io::stdin().lock().read_to_end(&mut content)?;
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
        let mut total_read = 0;

        loop {
            if total_read >= n {
                break;
            }
            let bytes_read = match &mut self.inner {
                FileInner::Read(reader) => reader.read(&mut buffer[total_read..])?,
                FileInner::ReadWrite(reader) => reader.read(&mut buffer[total_read..])?,
                FileInner::Stdin => io::stdin().lock().read(&mut buffer[total_read..])?,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "File not opened for reading",
                    ));
                }
            };
            if bytes_read == 0 {
                break; // EOF
            }
            total_read += bytes_read;
        }

        buffer.truncate(total_read);
        Ok(buffer)
    }

    /// Check if at EOF by reading 1 byte and putting it back via seek
    pub fn is_eof(&mut self) -> io::Result<bool> {
        use std::io::{Read, Seek, SeekFrom};
        let mut buf = [0u8; 1];
        match &mut self.inner {
            FileInner::Read(reader) => {
                let n = reader.read(&mut buf)?;
                if n == 0 { return Ok(true); }
                reader.seek(SeekFrom::Current(-1))?;
                Ok(false)
            }
            FileInner::ReadWrite(reader) => {
                let n = reader.read(&mut buf)?;
                if n == 0 { return Ok(true); }
                reader.seek(SeekFrom::Current(-1))?;
                Ok(false)
            }
            FileInner::Stdin => {
                // Can't easily check EOF on stdin without blocking
                Ok(false)
            }
            _ => Ok(true),
        }
    }

    /// Write operations
    pub fn write(&mut self, data: &str) -> io::Result<()> {
        // Convert from internal UTF-8/Latin-1 representation back to bytes
        // Each char's Unicode code point maps to its byte value (Latin-1)
        let bytes: Vec<u8> = data.chars().map(|c| c as u8).collect();
        self.write_bytes(&bytes)
    }

    pub fn write_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        match &mut self.inner {
            FileInner::Write(writer) => {
                writer.write_all(data)?;
                Ok(())
            }
            FileInner::ReadWrite(reader) => {
                reader.get_mut().write_all(data)?;
                Ok(())
            }
            FileInner::Stdout => {
                io::stdout().write_all(data)?;
                Ok(())
            }
            FileInner::Stderr => {
                io::stderr().write_all(data)?;
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
            FileInner::ReadWrite(reader) => reader.get_mut().flush(),
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

// LuaFile uses metatables for its Lua-visible API (read, write, etc.),
// so we only implement the minimal UserDataTrait for type identity and downcast.
impl UserDataTrait for LuaFile {
    fn type_name(&self) -> &'static str {
        "FILE*"
    }

    fn lua_gc(&mut self) {
        // Flush on GC — don't propagate errors
        let _ = self.flush();
    }

    fn lua_close(&mut self) {
        let _ = self.close();
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Create file metatable with methods
pub fn create_file_metatable(l: &mut LuaState) -> LuaResult<LuaValue> {
    let mt = l.create_table(0, 1)?;

    // Create __index table with methods
    let index_table = l.create_table(0, 7)?;

    // file:read([format])
    let read_key = l.create_string("read")?;
    l.raw_set(&index_table, read_key, LuaValue::cfunction(file_read));

    // file:write(...)
    let write_key = l.create_string("write")?;
    l.raw_set(&index_table, write_key, LuaValue::cfunction(file_write));

    // file:flush()
    let flush_key = l.create_string("flush")?;
    l.raw_set(&index_table, flush_key, LuaValue::cfunction(file_flush));

    // file:close()
    let close_key = l.create_string("close")?;
    l.raw_set(&index_table, close_key, LuaValue::cfunction(file_close));

    // file:lines([formats])
    let lines_key = l.create_string("lines")?;
    l.raw_set(&index_table, lines_key, LuaValue::cfunction(file_lines));

    // file:seek([whence [, offset]])
    let seek_key = l.create_string("seek")?;
    l.raw_set(&index_table, seek_key, LuaValue::cfunction(file_seek));

    // file:setvbuf(mode [, size])
    let setvbuf_key = l.create_string("setvbuf")?;
    l.raw_set(&index_table, setvbuf_key, LuaValue::cfunction(file_setvbuf));

    let index_key = l.create_string("__index")?;
    l.raw_set(&mt, index_key, index_table);

    // Set __close = file_gc_close (for <close> variables) - silently handles already-closed files
    let close_mm_key = l.create_string("__close")?;
    l.raw_set(&mt, close_mm_key, LuaValue::cfunction(file_gc_close));

    // Set __gc = file_gc_close (for garbage collection) - silently handles already-closed files
    let gc_key = l.create_string("__gc")?;
    l.raw_set(&mt, gc_key, LuaValue::cfunction(file_gc_close));

    // Set __tostring for file objects
    let tostring_key = l.create_string("__tostring")?;
    l.raw_set(&mt, tostring_key, LuaValue::cfunction(file_tostring));

    // Set __name = "FILE*" for type identification (luaT_objtypename)
    let name_key = l.create_string("__name")?;
    let name_val = l.create_string("FILE*")?;
    l.raw_set(&mt, name_key, name_val);

    Ok(mt)
}

/// file:read([format, ...])
fn file_read(l: &mut LuaState) -> LuaResult<usize> {
    // For method calls from Lua, arg 1 is self (file object)
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:read requires file handle".to_string()))?;

    // Collect all format arguments
    let mut formats: Vec<LuaValue> = Vec::new();
    let mut i = 2;
    loop {
        match l.get_arg(i) {
            Some(v) => formats.push(v),
            None => break,
        }
        i += 1;
    }
    // Default: read a line
    if formats.is_empty() {
        formats.push(LuaValue::nil()); // sentinel for default "l"
    }

    // Extract LuaFile from userdata using direct pointer access
    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            let mut nresults = 0;
            let mut success = true;
            
            for fmt in &formats {
                if !success {
                    // After first nil result, all subsequent are nil
                    l.push_value(LuaValue::nil())?;
                    nresults += 1;
                    continue;
                }
                
                // Helper macro-like: on IO error, return (nil, errmsg) like C Lua
                macro_rules! handle_read_err {
                    ($e:expr, $l:expr, $nresults:expr) => {{
                        $l.push_value(LuaValue::nil())?;
                        let msg = $l.create_string(&format!("{}", $e))?;
                        $l.push_value(msg)?;
                        let errno = $e.raw_os_error().unwrap_or(0);
                        $l.push_value(LuaValue::integer(errno as i64))?;
                        return Ok($nresults + 3);
                    }};
                }

                // Check if format is a number (byte count)
                if let Some(n) = fmt.as_integer() {
                    let n = n as usize;
                    if n == 0 {
                        // read(0) returns "" if not EOF, nil if EOF
                        match lua_file.is_eof() {
                            Ok(true) => {
                                l.push_value(LuaValue::nil())?;
                                success = false;
                            }
                            _ => {
                                let s = l.create_string("")?;
                                l.push_value(s)?;
                            }
                        }
                        nresults += 1;
                        continue;
                    }
                    match lua_file.read_bytes(n) {
                        Ok(bytes) => {
                            if bytes.is_empty() {
                                l.push_value(LuaValue::nil())?;
                                success = false;
                            } else {
                                let str_val = super::bytes_to_lua_value(l, bytes)?;
                                l.push_value(str_val)?;
                            }
                        }
                        Err(e) => handle_read_err!(e, l, nresults),
                    }
                    nresults += 1;
                    continue;
                }
                
                // Get format string (default "l" for nil sentinel)
                let format_str = fmt.as_str().map(|s| s.to_string())
                    .unwrap_or_else(|| "l".to_string());
                let format = format_str.strip_prefix('*').unwrap_or(&format_str);

                let first_char = format.chars().next().unwrap_or('l');
                let result: LuaValue = match first_char {
                    'l' => {
                        match lua_file.read_line() {
                            Ok(Some(line)) => l.create_string(&line)?,
                            Ok(None) => { success = false; LuaValue::nil() },
                            Err(e) => handle_read_err!(e, l, nresults),
                        }
                    }
                    'L' => {
                        match lua_file.read_line_with_newline() {
                            Ok(Some(line)) => l.create_string(&line)?,
                            Ok(None) => { success = false; LuaValue::nil() },
                            Err(e) => handle_read_err!(e, l, nresults),
                        }
                    }
                    'a' => {
                        match lua_file.read_all() {
                            Ok(content) => super::bytes_to_lua_value(l, content)?,
                            Err(e) => handle_read_err!(e, l, nresults),
                        }
                    }
                    'n' => {
                        match lua_file.read_number() {
                            Ok(Some(ReadNumberResult::Integer(n))) => LuaValue::integer(n),
                            Ok(Some(ReadNumberResult::Float(n))) => LuaValue::float(n),
                            Ok(None) => { success = false; LuaValue::nil() },
                            Err(e) => handle_read_err!(e, l, nresults),
                        }
                    }
                    _ => {
                        return Err(l.error(format!("invalid format: {}", format)));
                    }
                };

                l.push_value(result)?;
                nresults += 1;
            }
            
            return Ok(nresults);
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

                let write_result = match val.kind() {
                    LuaValueKind::String => {
                        if let Some(s) = val.as_str() {
                            lua_file.write(&s)
                        } else {
                            return Err(l.error("write expects strings or numbers".to_string()));
                        }
                    }
                    LuaValueKind::Binary => {
                        if let Some(b) = val.as_binary() {
                            lua_file.write_bytes(b)
                        } else {
                            return Err(l.error("write expects strings or numbers".to_string()));
                        }
                    }
                    LuaValueKind::Integer => {
                        if let Some(n) = val.as_integer() {
                            lua_file.write(&n.to_string())
                        } else {
                            return Err(l.error("write expects strings or numbers".to_string()));
                        }
                    }
                    LuaValueKind::Float => {
                        if let Some(n) = val.as_float() {
                            lua_file.write(&n.to_string())
                        } else {
                            return Err(l.error("write expects strings or numbers".to_string()));
                        }
                    }
                    _ => {
                        return Err(l.error("write expects strings or numbers".to_string()));
                    }
                };

                if let Err(e) = write_result {
                    // Return (nil, errmsg, errno) like C Lua
                    l.push_value(LuaValue::nil())?;
                    let msg = l.create_string(&format!("{}", e))?;
                    l.push_value(msg)?;
                    let errno = e.raw_os_error().unwrap_or(0);
                    l.push_value(LuaValue::integer(errno as i64))?;
                    return Ok(3);
                }

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

/// __close / __gc metamethod for files - validates arg type but silently handles already-closed files
fn file_gc_close(l: &mut LuaState) -> LuaResult<usize> {
    let file_val = match l.get_arg(1) {
        Some(v) => v,
        None => {
            let nil = LuaValue::nil();
            return Err(crate::stdlib::debug::arg_typeerror(l, 1, "FILE*", &nil));
        }
    };

    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            if !lua_file.is_closed() && !lua_file.is_std_stream() {
                let _ = lua_file.close();
            }
            return Ok(0);
        }
    }

    Err(crate::stdlib::debug::arg_typeerror(l, 1, "FILE*", &file_val))
}

/// file:close()
fn file_close(l: &mut LuaState) -> LuaResult<usize> {
    // For method calls from Lua, arg 1 is self (file object)
    let file_val = match l.get_arg(1) {
        Some(v) => v,
        None => {
            let nil = LuaValue::nil();
            return Err(crate::stdlib::debug::arg_typeerror(l, 1, "FILE*", &nil));
        }
    };

    // Extract LuaFile from userdata using direct pointer access
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
            if let Err(e) = lua_file.close() {
                return Err(l.error(format!("close error: {}", e)));
            }
            l.push_value(LuaValue::boolean(true))?;
            return Ok(1);
        }
    }

    Err(crate::stdlib::debug::arg_typeerror(l, 1, "FILE*", &file_val))
}

/// __tostring metamethod for file objects
fn file_tostring(l: &mut LuaState) -> LuaResult<usize> {
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("expected file handle".to_string()))?;

    if let Some(ud) = file_val.as_userdata_mut() {
        let data = ud.get_data_mut();
        if let Some(lua_file) = data.downcast_mut::<LuaFile>() {
            let s = if lua_file.is_closed() {
                "file (closed)".to_string()
            } else {
                format!("file ({:p})", ud as *const _)
            };
            let val = l.create_string(&s)?;
            l.push_value(val)?;
            return Ok(1);
        }
    }

    let val = l.create_string("file (closed)")?;
    l.push_value(val)?;
    Ok(1)
}

/// file:lines([formats]) - Returns an iterator for reading lines
fn file_lines(l: &mut LuaState) -> LuaResult<usize> {
    // Get file handle from self
    let file_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("file:lines requires file handle".to_string()))?;

    // Collect format arguments (start from arg 2)
    let mut formats: Vec<LuaValue> = Vec::new();
    let mut i = 2;
    loop {
        match l.get_arg(i) {
            Some(v) => formats.push(v),
            None => break,
        }
        i += 1;
    }

    // Create a callable state table with __call metamethod
    // file:lines() does NOT close the file when iteration ends
    let state_table = l.create_table(0, 5)?;
    let file_key = l.create_string("file")?;
    l.raw_set(&state_table, file_key, file_val);
    let closed_key = l.create_string("closed")?;
    l.raw_set(&state_table, closed_key, LuaValue::boolean(false));
    let noclose_key = l.create_string("noclose")?;
    l.raw_set(&state_table, noclose_key, LuaValue::boolean(true));

    // Store formats
    let fmts_table = l.create_table(formats.len(), 0)?;
    for (idx, fmt) in formats.iter().enumerate() {
        l.raw_seti(&fmts_table, (idx + 1) as i64, fmt.clone());
    }
    let fmts_key = l.create_string("fmts")?;
    l.raw_set(&state_table, fmts_key, fmts_table);
    let nfmts_key = l.create_string("nfmts")?;
    l.raw_set(&state_table, nfmts_key, LuaValue::integer(formats.len() as i64));

    // Create metatable with __call using the shared io_lines_call
    let mt = l.create_table(0, 1)?;
    let call_key = l.create_string("__call")?;
    l.raw_set(&mt, call_key, LuaValue::cfunction(super::io_lines_call));
    if let Some(t) = state_table.as_table_mut() {
        t.set_metatable(Some(mt));
    }

    l.push_value(state_table)?;
    Ok(1)
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
                FileInner::Write(writer) => writer.seek(seek_from),
                FileInner::ReadWrite(reader) => reader.seek(seek_from),
                FileInner::Closed => {
                    return Err(l.error("file is closed".to_string()));
                }
                FileInner::Stdin | FileInner::Stdout | FileInner::Stderr => {
                    // Seeking on std streams fails - return nil, msg, errno
                    l.push_value(LuaValue::nil())?;
                    let msg = l.create_string("cannot seek on standard stream")?;
                    l.push_value(msg)?;
                    l.push_value(LuaValue::integer(29))?; // ESPIPE
                    return Ok(3);
                }
            };

            match pos {
                Ok(position) => {
                    l.push_value(LuaValue::integer(position as i64))?;
                    return Ok(1);
                }
                Err(e) => {
                    // Return nil, msg, errno
                    l.push_value(LuaValue::nil())?;
                    let msg = l.create_string(&e.to_string())?;
                    l.push_value(msg)?;
                    let errno = e.raw_os_error().unwrap_or(0) as i64;
                    l.push_value(LuaValue::integer(errno))?;
                    return Ok(3);
                }
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
