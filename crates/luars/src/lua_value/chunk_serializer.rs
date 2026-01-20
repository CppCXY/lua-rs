// Chunk serializer/deserializer for string.dump/load
// Custom binary format for lua-rs bytecode

use super::{Chunk, LuaValue, UpvalueDesc};
use crate::Instruction;
use crate::gc::ObjectPool;
use std::io::{Cursor, Read};
use std::rc::Rc;

// Magic number for lua-rs bytecode (different from official Lua)
const LUARS_MAGIC: &[u8] = b"\x1bLuaRS";
const LUARS_VERSION: u8 = 1;

/// Serialize a Chunk to binary format (requires ObjectPool for string access)
pub fn serialize_chunk_with_pool(
    chunk: &Chunk,
    strip: bool,
    pool: &ObjectPool,
) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();

    // Write header
    buf.extend_from_slice(LUARS_MAGIC);
    buf.push(LUARS_VERSION);
    buf.push(if strip { 1 } else { 0 });

    // Write chunk data
    write_chunk(&mut buf, chunk, strip, pool)?;

    Ok(buf)
}

/// Serialize a Chunk to binary format (no string constants supported)
pub fn serialize_chunk(chunk: &Chunk, strip: bool) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();

    // Write header
    buf.extend_from_slice(LUARS_MAGIC);
    buf.push(LUARS_VERSION);
    buf.push(if strip { 1 } else { 0 });

    // Write chunk data without pool access (strings will be nil)
    write_chunk_no_pool(&mut buf, chunk, strip)?;

    Ok(buf)
}

/// Deserialize binary data to a Chunk
pub fn deserialize_chunk(data: &[u8]) -> Result<Chunk, String> {
    let mut cursor = Cursor::new(data);

    // Verify magic number
    let mut magic = [0u8; 6];
    cursor
        .read_exact(&mut magic)
        .map_err(|e| format!("failed to read magic: {}", e))?;
    if &magic != LUARS_MAGIC {
        return Err("not a lua-rs bytecode file".to_string());
    }

    // Read version
    let mut version = [0u8; 1];
    cursor
        .read_exact(&mut version)
        .map_err(|e| format!("failed to read version: {}", e))?;
    if version[0] != LUARS_VERSION {
        return Err(format!("unsupported bytecode version: {}", version[0]));
    }

    // Read strip flag (for future use)
    let mut stripped = [0u8; 1];
    cursor
        .read_exact(&mut stripped)
        .map_err(|_| "failed to read strip flag")?;

    // Read chunk data
    read_chunk(&mut cursor)
}

/// Deserialize binary data to a Chunk, returning string constants separately
pub fn deserialize_chunk_with_strings(
    data: &[u8],
) -> Result<(Chunk, Vec<(usize, String)>), String> {
    let mut cursor = Cursor::new(data);

    // Verify magic number
    let mut magic = [0u8; 6];
    cursor
        .read_exact(&mut magic)
        .map_err(|e| format!("failed to read magic: {}", e))?;
    if &magic != LUARS_MAGIC {
        return Err("not a lua-rs bytecode file".to_string());
    }

    // Read version
    let mut version = [0u8; 1];
    cursor
        .read_exact(&mut version)
        .map_err(|e| format!("failed to read version: {}", e))?;
    if version[0] != LUARS_VERSION {
        return Err(format!("unsupported bytecode version: {}", version[0]));
    }

    // Read strip flag
    let mut stripped = [0u8; 1];
    cursor
        .read_exact(&mut stripped)
        .map_err(|_| "failed to read strip flag")?;

    // Read chunk data with string collection
    let mut strings = Vec::new();
    let chunk = read_chunk_with_strings(&mut cursor, &mut strings)?;
    Ok((chunk, strings))
}

fn write_chunk(
    buf: &mut Vec<u8>,
    chunk: &Chunk,
    strip: bool,
    pool: &ObjectPool,
) -> Result<(), String> {
    // Write code
    write_u32(buf, chunk.code.len() as u32);
    for &instr in &chunk.code {
        write_u32(buf, instr.as_u32());
    }

    // Write constants
    write_u32(buf, chunk.constants.len() as u32);
    for constant in &chunk.constants {
        write_constant_with_pool(buf, constant)?;
    }

    // Write metadata
    write_u32(buf, chunk.upvalue_count as u32);
    write_u32(buf, chunk.param_count as u32);
    buf.push(if chunk.is_vararg { 1 } else { 0 });
    write_u32(buf, chunk.max_stack_size as u32);

    // Write upvalue descriptors
    write_u32(buf, chunk.upvalue_descs.len() as u32);
    for desc in &chunk.upvalue_descs {
        buf.push(if desc.is_local { 1 } else { 0 });
        write_u32(buf, desc.index);
    }

    // Write child prototypes
    write_u32(buf, chunk.child_protos.len() as u32);
    for child in &chunk.child_protos {
        write_chunk(buf, child, strip, pool)?;
    }

    // Write debug info (if not stripped)
    if strip {
        write_u32(buf, 0); // no source name
        write_u32(buf, 0); // no locals
        write_u32(buf, 0); // no line info
    } else {
        if let Some(ref name) = chunk.source_name {
            write_string(buf, name);
        } else {
            write_u32(buf, 0);
        }

        write_u32(buf, chunk.locals.len() as u32);
        for local in &chunk.locals {
            write_string(buf, local);
        }

        write_u32(buf, chunk.line_info.len() as u32);
        for &line in &chunk.line_info {
            write_u32(buf, line);
        }
    }

    Ok(())
}

fn write_chunk_no_pool(buf: &mut Vec<u8>, chunk: &Chunk, strip: bool) -> Result<(), String> {
    // Write code
    write_u32(buf, chunk.code.len() as u32);
    for &instr in &chunk.code {
        write_u32(buf, instr.as_u32());
    }

    // Write constants (without pool, strings become nil)
    write_u32(buf, chunk.constants.len() as u32);
    for constant in &chunk.constants {
        write_constant_no_pool(buf, constant)?;
    }

    // Write metadata
    write_u32(buf, chunk.upvalue_count as u32);
    write_u32(buf, chunk.param_count as u32);
    buf.push(if chunk.is_vararg { 1 } else { 0 });
    write_u32(buf, chunk.max_stack_size as u32);

    // Write upvalue descriptors
    write_u32(buf, chunk.upvalue_descs.len() as u32);
    for desc in &chunk.upvalue_descs {
        buf.push(if desc.is_local { 1 } else { 0 });
        write_u32(buf, desc.index);
    }

    // Write child prototypes
    write_u32(buf, chunk.child_protos.len() as u32);
    for child in &chunk.child_protos {
        write_chunk_no_pool(buf, child, strip)?;
    }

    // Write debug info
    if strip {
        write_u32(buf, 0);
        write_u32(buf, 0);
        write_u32(buf, 0);
    } else {
        if let Some(ref name) = chunk.source_name {
            write_string(buf, name);
        } else {
            write_u32(buf, 0);
        }

        write_u32(buf, chunk.locals.len() as u32);
        for local in &chunk.locals {
            write_string(buf, local);
        }

        write_u32(buf, chunk.line_info.len() as u32);
        for &line in &chunk.line_info {
            write_u32(buf, line);
        }
    }

    Ok(())
}

fn read_chunk(cursor: &mut Cursor<&[u8]>) -> Result<Chunk, String> {
    // Read code
    let code_len = read_u32(cursor)? as usize;
    let mut code = Vec::with_capacity(code_len);
    for _ in 0..code_len {
        code.push(Instruction::from_u32(read_u32(cursor)?));
    }

    // Read constants
    let const_len = read_u32(cursor)? as usize;
    let mut constants = Vec::with_capacity(const_len);
    for _ in 0..const_len {
        constants.push(read_constant(cursor)?);
    }

    // Read metadata
    let upvalue_count = read_u32(cursor)? as usize;
    let param_count = read_u32(cursor)? as usize;
    let is_vararg = read_u8(cursor)? != 0;
    let max_stack_size = read_u32(cursor)? as usize;

    // Read upvalue descriptors
    let desc_len = read_u32(cursor)? as usize;
    let mut upvalue_descs = Vec::with_capacity(desc_len);
    for _ in 0..desc_len {
        let is_local = read_u8(cursor)? != 0;
        let index = read_u32(cursor)?;
        upvalue_descs.push(UpvalueDesc { is_local, index });
    }

    // Read child prototypes
    let child_len = read_u32(cursor)? as usize;
    let mut child_protos = Vec::with_capacity(child_len);
    for _ in 0..child_len {
        child_protos.push(Rc::new(read_chunk(cursor)?));
    }

    // Read debug info
    let source_name = read_optional_string(cursor)?;

    let locals_len = read_u32(cursor)? as usize;
    let mut locals = Vec::with_capacity(locals_len);
    for _ in 0..locals_len {
        locals.push(read_string(cursor)?);
    }

    let line_len = read_u32(cursor)? as usize;
    let mut line_info = Vec::with_capacity(line_len);
    for _ in 0..line_len {
        line_info.push(read_u32(cursor)?);
    }

    Ok(Chunk {
        code,
        constants,
        locals,
        upvalue_count,
        param_count,
        is_vararg,
        needs_vararg_table: false,
        use_hidden_vararg: false,
        max_stack_size,
        child_protos,
        upvalue_descs,
        source_name,
        line_info,
        linedefined: 0,
        lastlinedefined: 0,
    })
}

// Constant type tags
const TAG_NIL: u8 = 0;
const TAG_BOOL_FALSE: u8 = 1;
const TAG_BOOL_TRUE: u8 = 2;
const TAG_INTEGER: u8 = 3;
const TAG_FLOAT: u8 = 4;
const TAG_STRING: u8 = 5;

fn write_constant_with_pool(buf: &mut Vec<u8>, value: &LuaValue) -> Result<(), String> {
    if value.is_nil() {
        buf.push(TAG_NIL);
    } else if let Some(b) = value.as_boolean() {
        buf.push(if b { TAG_BOOL_TRUE } else { TAG_BOOL_FALSE });
    } else if let Some(i) = value.as_integer() {
        buf.push(TAG_INTEGER);
        write_i64(buf, i);
    } else if let Some(f) = value.as_float() {
        buf.push(TAG_FLOAT);
        write_f64(buf, f);
    } else if let Some(lua_string) = value.as_str() {
        buf.push(TAG_STRING);
        write_string(buf, lua_string);
    } else {
        buf.push(TAG_NIL);
    }
    Ok(())
}

fn write_constant_no_pool(buf: &mut Vec<u8>, value: &LuaValue) -> Result<(), String> {
    if value.is_nil() {
        buf.push(TAG_NIL);
    } else if let Some(b) = value.as_boolean() {
        buf.push(if b { TAG_BOOL_TRUE } else { TAG_BOOL_FALSE });
    } else if let Some(i) = value.as_integer() {
        buf.push(TAG_INTEGER);
        write_i64(buf, i);
    } else if let Some(f) = value.as_float() {
        buf.push(TAG_FLOAT);
        write_f64(buf, f);
    } else {
        // Without pool access, we can't serialize strings
        buf.push(TAG_NIL);
    }
    Ok(())
}

/// Read a constant, collecting string data for later VM processing
fn read_constant_with_strings(
    cursor: &mut Cursor<&[u8]>,
    const_index: usize,
    strings: &mut Vec<(usize, String)>,
) -> Result<LuaValue, String> {
    let tag = read_u8(cursor)?;
    match tag {
        TAG_NIL => Ok(LuaValue::nil()),
        TAG_BOOL_FALSE => Ok(LuaValue::boolean(false)),
        TAG_BOOL_TRUE => Ok(LuaValue::boolean(true)),
        TAG_INTEGER => Ok(LuaValue::integer(read_i64(cursor)?)),
        TAG_FLOAT => Ok(LuaValue::number(read_f64(cursor)?)),
        TAG_STRING => {
            let s = read_string(cursor)?;
            // Store string for later, use nil as placeholder
            strings.push((const_index, s));
            Ok(LuaValue::nil()) // Will be replaced by VM
        }
        _ => Err(format!("unknown constant tag: {}", tag)),
    }
}

fn read_constant(cursor: &mut Cursor<&[u8]>) -> Result<LuaValue, String> {
    let tag = read_u8(cursor)?;
    match tag {
        TAG_NIL => Ok(LuaValue::nil()),
        TAG_BOOL_FALSE => Ok(LuaValue::boolean(false)),
        TAG_BOOL_TRUE => Ok(LuaValue::boolean(true)),
        TAG_INTEGER => Ok(LuaValue::integer(read_i64(cursor)?)),
        TAG_FLOAT => Ok(LuaValue::number(read_f64(cursor)?)),
        TAG_STRING => {
            // Skip the string data, return nil as placeholder
            let len = read_u32(cursor)? as usize;
            let mut buf = vec![0u8; len];
            cursor
                .read_exact(&mut buf)
                .map_err(|e| format!("read error: {}", e))?;
            Ok(LuaValue::nil())
        }
        _ => Err(format!("unknown constant tag: {}", tag)),
    }
}

fn read_chunk_with_strings(
    cursor: &mut Cursor<&[u8]>,
    strings: &mut Vec<(usize, String)>,
) -> Result<Chunk, String> {
    // Read code
    let code_len = read_u32(cursor)? as usize;
    let mut code = Vec::with_capacity(code_len);
    for _ in 0..code_len {
        code.push(Instruction::from_u32(read_u32(cursor)?));
    }

    // Read constants with string collection
    let const_len = read_u32(cursor)? as usize;
    let mut constants = Vec::with_capacity(const_len);
    for i in 0..const_len {
        constants.push(read_constant_with_strings(cursor, i, strings)?);
    }

    // Read metadata
    let upvalue_count = read_u32(cursor)? as usize;
    let param_count = read_u32(cursor)? as usize;
    let is_vararg = read_u8(cursor)? != 0;
    let max_stack_size = read_u32(cursor)? as usize;

    // Read upvalue descriptors
    let desc_len = read_u32(cursor)? as usize;
    let mut upvalue_descs = Vec::with_capacity(desc_len);
    for _ in 0..desc_len {
        let is_local = read_u8(cursor)? != 0;
        let index = read_u32(cursor)?;
        upvalue_descs.push(UpvalueDesc { is_local, index });
    }

    // Read child prototypes
    let child_len = read_u32(cursor)? as usize;
    let mut child_protos = Vec::with_capacity(child_len);
    for _ in 0..child_len {
        child_protos.push(Rc::new(read_chunk_with_strings(cursor, strings)?));
    }

    // Read debug info
    let source_name = read_optional_string(cursor)?;

    let locals_len = read_u32(cursor)? as usize;
    let mut locals = Vec::with_capacity(locals_len);
    for _ in 0..locals_len {
        locals.push(read_string(cursor)?);
    }

    let line_len = read_u32(cursor)? as usize;
    let mut line_info = Vec::with_capacity(line_len);
    for _ in 0..line_len {
        line_info.push(read_u32(cursor)?);
    }

    Ok(Chunk {
        code,
        constants,
        locals,
        upvalue_count,
        param_count,
        is_vararg,
        needs_vararg_table: false,
        use_hidden_vararg: false,
        max_stack_size,
        child_protos,
        upvalue_descs,
        source_name,
        line_info,
        linedefined: 0,
        lastlinedefined: 0,
    })
}

// Helper functions for binary I/O
fn write_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_i64(buf: &mut Vec<u8>, value: i64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_f64(buf: &mut Vec<u8>, value: f64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    write_u32(buf, bytes.len() as u32);
    buf.extend_from_slice(bytes);
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8, String> {
    let mut buf = [0u8; 1];
    cursor
        .read_exact(&mut buf)
        .map_err(|e| format!("read error: {}", e))?;
    Ok(buf[0])
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    cursor
        .read_exact(&mut buf)
        .map_err(|e| format!("read error: {}", e))?;
    Ok(u32::from_le_bytes(buf))
}

fn read_i64(cursor: &mut Cursor<&[u8]>) -> Result<i64, String> {
    let mut buf = [0u8; 8];
    cursor
        .read_exact(&mut buf)
        .map_err(|e| format!("read error: {}", e))?;
    Ok(i64::from_le_bytes(buf))
}

fn read_f64(cursor: &mut Cursor<&[u8]>) -> Result<f64, String> {
    let mut buf = [0u8; 8];
    cursor
        .read_exact(&mut buf)
        .map_err(|e| format!("read error: {}", e))?;
    Ok(f64::from_le_bytes(buf))
}

fn read_string(cursor: &mut Cursor<&[u8]>) -> Result<String, String> {
    let len = read_u32(cursor)? as usize;
    let mut buf = vec![0u8; len];
    cursor
        .read_exact(&mut buf)
        .map_err(|e| format!("read error: {}", e))?;
    String::from_utf8(buf).map_err(|e| format!("invalid utf8: {}", e))
}

fn read_optional_string(cursor: &mut Cursor<&[u8]>) -> Result<Option<String>, String> {
    let len = read_u32(cursor)? as usize;
    if len == 0 {
        return Ok(None);
    }
    let mut buf = vec![0u8; len];
    cursor
        .read_exact(&mut buf)
        .map_err(|e| format!("read error: {}", e))?;
    Ok(Some(
        String::from_utf8(buf).map_err(|e| format!("invalid utf8: {}", e))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_empty_chunk() {
        let chunk = Chunk::new();
        let bytes = serialize_chunk(&chunk, false).unwrap();
        let restored = deserialize_chunk(&bytes).unwrap();
        assert_eq!(restored.code.len(), 0);
        assert_eq!(restored.constants.len(), 0);
    }
}
