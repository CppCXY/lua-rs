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
        return Err(
            l.error("bad argument #1 to 'create' (array size must be non-negative)".to_string())
        );
    }
    if nhash < 0 {
        return Err(
            l.error("bad argument #2 to 'create' (hash size must be non-negative)".to_string())
        );
    }

    // Check for overflow (INT_MAX in Lua is i32::MAX)
    if narray > i32::MAX as i64 {
        return Err(l.error("bad argument #1 to 'create' (array size too large)".to_string()));
    }
    if nhash > i32::MAX as i64 {
        return Err(l.error("bad argument #2 to 'create' (hash size too large)".to_string()));
    }

    // Create table with pre-allocated sizes
    let table = l.create_table(narray as usize, nhash as usize)?;
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
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                return Err(l.error("bad argument #2 to 'concat' (string expected)".to_string()));
            }
        }
        None => "".to_string(),
    };

    let Some(table) = table_val.as_table() else {
        return Err(l.error("bad argument #1 to 'concat' (table expected)".to_string()));
    };

    // Get arguments
    let i = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let j = l
        .get_arg(4)
        .and_then(|v| v.as_integer())
        .unwrap_or_else(|| table.len() as i64);

    let mut parts = Vec::new();
    for idx in i..=j {
        if let Some(value) = table.raw_geti(idx) {
            if let Some(s) = value.as_str() {
                parts.push(s.to_string());
            } else {
                let msg = format!("bad value at index {} in 'concat' (string expected)", idx);
                return Err(l.error(msg));
            }
        }
    }

    // Concat the parts with separator
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

    let Some(table) = table_val.as_table() else {
        return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
    };

    let len = table.len();

    if argc == 2 {
        // table.insert(list, value) - append at end
        let value = l
            .get_arg(2)
            .ok_or_else(|| l.error("bad argument #2 to 'insert' (value expected)".to_string()))?;
        let Some(table_ref) = table_val.as_table_mut() else {
            return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
        };
        // Append at position len + 1 (1-based indexing)
        match table_ref.insert_array_at(len as i64 + 1, value) {
            Ok(new_key) => {
                if new_key {
                    // New key inserted - run GC barrier
                    if value.is_collectable() {
                        l.vm_mut().gc.barrier_back(table_val.as_gc_ptr().unwrap());
                    }
                }
            }
            Err(e) => {
                return Err(l.error(format!("error inserting into table: {}", e)));
            }
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

        if pos < 1 || pos > len as i64 + 1 {
            return Err(l.error("bad argument #2 to 'insert' (position out of bounds)".to_string()));
        }
        let Some(table_ref) = table_val.as_table_mut() else {
            return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
        };
        match table_ref.insert_array_at(pos, value) {
            Ok(new_key) => {
                if new_key {
                    // New key inserted - run GC barrier
                    if value.is_collectable() {
                        l.vm_mut().gc.barrier_back(table_val.as_gc_ptr().unwrap());
                    }
                }
            }
            Err(e) => {
                return Err(l.error(format!("error inserting into table: {}", e)));
            }
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

    let Some(table) = table_val.as_table() else {
        return Err(l.error("bad argument #1 to 'remove' (table expected)".to_string()));
    };

    let len = table.len();

    if len == 0 {
        l.push_value(LuaValue::nil())?;
        return Ok(1);
    }

    let pos = l
        .get_arg(2)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    if pos < 1 || pos > len as i64 {
        return Err(l.error("bad argument #2 to 'remove' (position out of bounds)".to_string()));
    }

    let Some(table_ref) = table_val.as_table_mut() else {
        return Err(l.error("bad argument #1 to 'remove' (table expected)".to_string()));
    };

    // Remove the element using the proper method
    let removed = match table_ref.remove_array_at(pos) {
        Ok(val) => val,
        Err(e) => {
            return Err(l.error(format!("error removing from table: {}", e)));
        }
    };

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

    let dst_value = l.get_arg(5).unwrap_or_else(|| src_val.clone());

    // Copy elements
    let Some(src_ref) = src_val.as_table() else {
        return Err(l.error("bad argument #1 to 'move' (table expected)".to_string()));
    };

    let mut values = Vec::new();
    for i in f..=e {
        let val = src_ref.raw_geti(i).unwrap_or(LuaValue::nil());
        values.push(val);
    }

    for (offset, val) in values.into_iter().enumerate() {
        l.raw_seti(&dst_value, t + offset as i64, val);
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

    let Some(table_ref) = table_val.as_table() else {
        return Err(l.error("bad argument #1 to 'unpack' (table expected)".to_string()));
    };

    // Get arguments
    let len = table_ref.len();
    let i = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(1);
    let j = l
        .get_arg(3)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    // Handle empty range
    if i > j {
        return Ok(0);
    }

    // Collect all values
    let mut values = Vec::new();
    for idx in i..=j {
        values.push(table_ref.raw_geti(idx).unwrap_or(LuaValue::nil()));
    }

    // Push values after vm borrow ends
    let count = values.len();
    for val in values {
        l.push_value(val)?;
    }

    Ok(count)
}
