// Table library
// Implements: concat, insert, move, pack, remove, sort, unpack

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};
use crate::stdlib::sort_table::table_sort;

pub fn create_table_lib() -> LibraryModule {
    crate::lib_module!("table", {
        "concat" => table_concat,
        "create" => table_create,
        "insert" => table_insert,
        "move" => table_move,
        "pack" => table_pack,
        "remove" => table_remove,
        "sort" => table_sort,
        "unpack" => table_unpack,
    })
}

/// table.create(narray [, nhash]) - Create a pre-allocated table (Lua 5.5)
fn table_create(l: &mut LuaState) -> LuaResult<usize> {
    let narray = l.get_arg(1).and_then(|v| v.as_integer()).unwrap_or(0);

    let nhash = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(0);

    // Validate arguments
    if narray < 0 {
        return Err(l.error("bad argument #1 to 'create' (out of range)".to_string()));
    }
    if nhash < 0 {
        return Err(l.error("bad argument #2 to 'create' (out of range)".to_string()));
    }

    // Check for overflow (INT_MAX in Lua is i32::MAX)
    if narray > i32::MAX as i64 {
        return Err(l.error("bad argument #1 to 'create' (out of range)".to_string()));
    }
    if nhash > i32::MAX as i64 {
        return Err(l.error("bad argument #2 to 'create' (out of range)".to_string()));
    }

    // Limit to reasonable sizes to avoid allocation panics
    let max_safe = 1 << 24; // ~16M elements
    let na = std::cmp::min(narray as usize, max_safe);
    let nh = if nhash as usize > max_safe {
        return Err(l.error("table overflow".to_string()));
    } else {
        nhash as usize
    };

    // Create table with pre-allocated sizes
    let table = l.create_table(na, nh)?;
    l.push_value(table)?;
    Ok(1)
}

/// table.concat(list [, sep [, i [, j]]]) - Concatenate table elements
fn table_concat(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'concat' (table expected)".to_string()))?;

    let sep_value = l.get_arg(2);
    let sep = match sep_value {
        Some(v) => {
            if v.is_nil() {
                "".to_string()
            } else if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                return Err(l.error("bad argument #2 to 'concat' (string expected)".to_string()));
            }
        }
        None => "".to_string(),
    };

    if !table_val.is_table() {
        return Err(l.error("bad argument #1 to 'concat' (table expected)".to_string()));
    }

    let table = table_val.as_table().unwrap();

    let i = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let j = l.get_arg(4).and_then(|v| v.as_integer()).unwrap_or(table.len() as i64);

    // If i > j, return empty string immediately
    if i > j {
        let result = l.create_string("")?;
        l.push_value(result)?;
        return Ok(1);
    }

    let mut parts = Vec::new();
    for idx in i..=j {
        let value = table.raw_geti(idx).unwrap_or(LuaValue::nil());
        if let Some(s) = value.as_str() {
            parts.push(s.to_string());
        } else if let Some(ival) = value.as_integer() {
            parts.push(format!("{}", ival));
        } else if let Some(f) = value.as_number() {
            if f == f.floor() && f.abs() < 1e15 && !f.is_infinite() {
                parts.push(format!("{:.1}", f));
            } else {
                parts.push(format!("{}", f));
            }
        } else {
            let msg = format!("invalid value (at index {}) in table for 'concat'", idx);
            return Err(l.error(msg));
        }
    }

    let result = l.create_string(&parts.join(&sep))?;
    l.push_value(result)?;
    Ok(1)
}

/// table.insert(list, [pos,] value) - Insert element
fn table_insert(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'insert' (table expected)".to_string()))?;
    let argc = l.arg_count();

    if !table_val.is_table() {
        return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
    }

    let table = table_val.as_table_mut().unwrap();
    let len = table.len() as i64;

    if argc == 2 {
        // table.insert(list, value) - append at end
        let value = l
            .get_arg(2)
            .ok_or_else(|| l.error("bad argument #2 to 'insert' (value expected)".to_string()))?;
        table.raw_seti(len.wrapping_add(1), value);
        // GC write barrier: table was modified directly, bypass LuaVM::raw_seti barrier
        if let Some(gc_ptr) = table_val.as_gc_ptr() {
            l.gc_barrier_back(gc_ptr);
        }
    } else if argc == 3 {
        // table.insert(list, pos, value)
        let pos = l
            .get_arg(2)
            .ok_or_else(|| l.error("bad argument #2 to 'insert' (number expected)".to_string()))?
            .as_integer()
            .ok_or_else(|| l.error("bad argument #2 to 'insert' (number expected)".to_string()))?;

        let value = l
            .get_arg(3)
            .ok_or_else(|| l.error("bad argument #3 to 'insert' (value expected)".to_string()))?;

        if pos < 1 || pos > len + 1 {
            return Err(l.error("bad argument #2 to 'insert' (position out of bounds)".to_string()));
        }

        // Shift elements up: t[i+1] = t[i] for i = len down to pos
        let table = table_val.as_table_mut().unwrap();
        table.insert_array_at(pos, value)?;
        // GC write barrier: table was modified directly
        if let Some(gc_ptr) = table_val.as_gc_ptr() {
            l.gc_barrier_back(gc_ptr);
        }
    } else {
        return Err(l.error("wrong number of arguments to 'insert'".to_string()));
    }

    Ok(0)
}

/// table.remove(list [, pos]) - Remove element
fn table_remove(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'remove' (table expected)".to_string()))?;

    if !table_val.is_table() {
        return Err(l.error("bad argument #1 to 'remove' (table expected)".to_string()));
    }

    let table = table_val.as_table_mut().unwrap();
    let len = table.len() as i64;

    // Default pos = #t (like C Lua: luaL_optinteger(L, 2, size))
    let has_pos_arg = l.get_arg(2).is_some();
    let pos = l
        .get_arg(2)
        .and_then(|v| v.as_integer())
        .unwrap_or(len);

    // Only validate pos if explicitly given (C Lua: "if (pos != size)")
    if has_pos_arg && pos != len {
        if pos < 1 || pos > len.wrapping_add(1) {
            return Err(
                l.error("bad argument #2 to 'remove' (position out of bounds)".to_string())
            );
        }
    }

    // Get the value at pos
    let removed = table.raw_geti(pos).unwrap_or(LuaValue::nil());

    // Shift elements down: t[i] = t[i+1] for i = pos to len-1
    let mut i = pos;
    while i < len {
        let next_val = table.raw_geti(i.wrapping_add(1)).unwrap_or(LuaValue::nil());
        table.raw_seti(i, next_val);
        i += 1;
    }

    // Remove the last entry: t[len] = nil
    table.raw_seti(i, LuaValue::nil());

    // GC write barrier: table was modified directly (shifted elements may be collectable)
    if let Some(gc_ptr) = table_val.as_gc_ptr() {
        l.gc_barrier_back(gc_ptr);
    }

    l.push_value(removed)?;
    Ok(1)
}

/// table.move(a1, f, e, t [, a2]) - Move elements
fn table_move(l: &mut LuaState) -> LuaResult<usize> {
    let src_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'move' (table expected)".to_string()))?;

    let f = l
        .get_arg(2)
        .ok_or_else(|| l.error("bad argument #2 to 'move' (number expected)".to_string()))?
        .as_integer()
        .ok_or_else(|| l.error("bad argument #2 to 'move' (number expected)".to_string()))?;

    let e = l
        .get_arg(3)
        .ok_or_else(|| l.error("bad argument #3 to 'move' (number expected)".to_string()))?
        .as_integer()
        .ok_or_else(|| l.error("bad argument #3 to 'move' (number expected)".to_string()))?;

    let t = l
        .get_arg(4)
        .ok_or_else(|| l.error("bad argument #4 to 'move' (number expected)".to_string()))?
        .as_integer()
        .ok_or_else(|| l.error("bad argument #4 to 'move' (number expected)".to_string()))?;

    let dst_value = l.get_arg(5).unwrap_or(src_val);

    let src_table = src_val
        .as_table()
        .ok_or_else(|| l.error("bad argument #1 to 'move' (table expected)".to_string()))?;

    // Copy elements from source using raw access
    let mut values = Vec::new();
    for i in f..=e {
        let val = src_table.raw_geti(i).unwrap_or(LuaValue::nil());
        values.push(val);
    }

    // Write to destination using raw access
    let dst_table = dst_value
        .as_table_mut()
        .ok_or_else(|| l.error("bad argument #5 to 'move' (table expected)".to_string()))?;
    for (offset, val) in values.into_iter().enumerate() {
        dst_table.raw_seti(t.wrapping_add(offset as i64), val);
    }

    // GC write barrier: destination table was modified directly
    if let Some(gc_ptr) = dst_value.as_gc_ptr() {
        l.gc_barrier_back(gc_ptr);
    }

    l.push_value(dst_value)?;
    Ok(1)
}

/// table.pack(...) - Pack values into table
fn table_pack(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();
    let table = l.create_table(args.len(), 1)?;

    // Set 'n' field
    let n_key = l.create_string("n")?;
    if !table.is_table() {
        return Err(l.error("failed to create table".to_string()));
    };

    for (i, arg) in args.iter().enumerate() {
        l.raw_seti(&table, i as i64 + 1, arg.clone());
    }

    l.raw_set(&table, n_key, LuaValue::integer(args.len() as i64));
    l.push_value(table)?;
    Ok(1)
}

/// table.unpack(list [, i [, j]]) - Unpack table into values
/// OPTIMIZED: Pre-allocate Vec and use direct array access when possible
fn table_unpack(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'unpack' (table expected)".to_string()))?;

    if !table_val.is_table() {
        return Err(l.error("bad argument #1 to 'unpack' (table expected)".to_string()));
    }

    let table = table_val.as_table().unwrap();

    let i = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1);
    let j = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(table.len() as i64);

    // Handle empty range
    if i > j {
        return Ok(0);
    }

    // Check for excessive range (Lua 5.5: "too many results to unpack")
    let count = (j as u64).wrapping_sub(i as u64).wrapping_add(1);
    if count > 1_000_000 {
        return Err(l.error("too many results to unpack".to_string()));
    }

    // Collect all values using raw access
    let mut values = Vec::with_capacity(count as usize);
    for idx in i..=j {
        let val = table.raw_geti(idx).unwrap_or(LuaValue::nil());
        values.push(val);
    }

    // Push values
    let count = values.len();
    for val in values {
        l.push_value(val)?;
    }

    Ok(count)
}
