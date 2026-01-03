// Table library
// Implements: concat, insert, move, pack, remove, sort, unpack

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};

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
    let table_val = {
        let vm = l.vm_mut();
        let table = vm.object_pool.create_table(narray as usize, nhash as usize);
        LuaValue::table(table)
    };

    l.push_value(table_val)?;
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
            if let Some(string_id) = v.as_string_id() {
                let vm = l.vm_mut();
                if let Some(s) = vm.object_pool.get_string(string_id) {
                    s.as_str().to_string()
                } else {
                    return Err(
                        l.error("bad argument #2 to 'concat' (string expected)".to_string())
                    );
                }
            } else {
                return Err(l.error("bad argument #2 to 'concat' (string expected)".to_string()));
            }
        }
        None => "".to_string(),
    };

    let Some(table_id) = table_val.as_table_id() else {
        return Err(l.error("bad argument #1 to 'concat' (table expected)".to_string()));
    };

    // Get arguments before vm_mut borrow
    let i = l.get_arg(3).and_then(|v| v.as_integer()).unwrap_or(1);
    let j_opt = l.get_arg(4).and_then(|v| v.as_integer());

    let parts = {
        let vm = l.vm_mut();
        let Some(table_borrowed) = vm.object_pool.get_table(table_id) else {
            let _ = vm; // Explicitly end borrow
            return Err(l.error("bad argument #1 to 'concat' (table expected)".to_string()));
        };
        let len = table_borrowed.len();
        let j = j_opt.unwrap_or(len as i64);

        let mut parts = Vec::new();
        for idx in i..=j {
            if let Some(value) = table_borrowed.get_int(idx) {
                if let Some(string_id) = value.as_string_id() {
                    if let Some(s) = vm.object_pool.get_string(string_id) {
                        parts.push(s.as_str().to_string());
                    } else {
                        let msg =
                            format!("bad value at index {} in 'concat' (string expected)", idx);
                        let _ = vm;
                        return Err(l.error(msg));
                    }
                } else {
                    let msg = format!("bad value at index {} in 'concat' (string expected)", idx);
                    let _ = vm;
                    return Err(l.error(msg));
                }
            }
        }
        parts
    };

    // Concat the parts with separator
    let result = l.create_string(&parts.join(&sep));
    l.push_value(result)?;
    Ok(1)
}

/// table.insert(list, [pos,] value) - Insert element
fn table_insert(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'insert' (table expected)".to_string()))?;
    let argc = l.arg_count();

    let Some(table_id) = table_val.as_table_id() else {
        return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
    };

    let len = {
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
        };
        table_ref.len()
    };

    if argc == 2 {
        // table.insert(list, value) - append at end
        let value = l
            .get_arg(2)
            .ok_or_else(|| l.error("bad argument #2 to 'insert' (value expected)".to_string()))?;
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
        };
        match table_ref.insert_array_at(len, value) {
            Ok(_) => {}
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
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(l.error("bad argument #1 to 'insert' (table expected)".to_string()));
        };
        match table_ref.insert_array_at(pos as usize - 1, value) {
            Ok(_) => {}
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

    let Some(table_id) = table_val.as_table_id() else {
        return Err(l.error("bad argument #1 to 'remove' (table expected)".to_string()));
    };

    let len = {
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(l.error("bad argument #1 to 'remove' (table expected)".to_string()));
        };
        table_ref.len()
    };

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

    let vm = l.vm_mut();
    let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
        return Err(l.error("bad argument #1 to 'remove' (table expected)".to_string()));
    };

    // Get the value to remove
    let removed = table_ref.get_int(pos).unwrap_or(LuaValue::nil());

    // Shift all elements after pos down by one
    for i in pos..len as i64 {
        let next_val = table_ref.get_int(i + 1).unwrap_or(LuaValue::nil());
        table_ref.set_int(i, next_val);
    }

    // Clear the last element
    table_ref.set_int(len as i64, LuaValue::nil());

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
    let mut values = Vec::new();
    {
        let Some(src_id) = src_val.as_table_id() else {
            return Err(l.error("bad argument #1 to 'move' (table expected)".to_string()));
        };
        let vm = l.vm_mut();
        let Some(src_ref) = vm.object_pool.get_table(src_id) else {
            return Err(l.error("bad argument #1 to 'move' (table expected)".to_string()));
        };

        for i in f..=e {
            let val = src_ref.get_int(i).unwrap_or(LuaValue::nil());
            values.push(val);
        }
    }

    {
        let Some(dst_id) = dst_value.as_table_id() else {
            return Err(l.error("bad argument #5 to 'move' (table expected)".to_string()));
        };
        let vm = l.vm_mut();
        let Some(dst_ref) = vm.object_pool.get_table_mut(dst_id) else {
            return Err(l.error("bad argument #5 to 'move' (table expected)".to_string()));
        };
        for (offset, val) in values.into_iter().enumerate() {
            dst_ref.set_int(t + offset as i64, val);
        }
    }

    l.push_value(dst_value)?;
    Ok(1)
}

/// table.pack(...) - Pack values into table
fn table_pack(l: &mut LuaState) -> LuaResult<usize> {
    let args = l.get_args();
    let table = l.create_table(args.len(), 1);

    // Set 'n' field
    let n_key = l.create_string("n");
    let Some(table_id) = table.as_table_id() else {
        return Err(l.error("failed to create table".to_string()));
    };
    let vm = l.vm_mut();
    let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
        return Err(l.error("failed to create table".to_string()));
    };

    for (i, arg) in args.iter().enumerate() {
        table_ref.set_int(i as i64 + 1, arg.clone());
    }

    table_ref.raw_set(n_key, LuaValue::integer(args.len() as i64));
    l.push_value(table)?;
    Ok(1)
}

/// table.unpack(list [, i [, j]]) - Unpack table into values
/// OPTIMIZED: Pre-allocate Vec and use direct array access when possible
fn table_unpack(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'unpack' (table expected)".to_string()))?;

    let Some(table_id) = table_val.as_table_id() else {
        return Err(l.error("bad argument #1 to 'unpack' (table expected)".to_string()));
    };

    // Get arguments before vm_mut borrow
    let i_opt = l.get_arg(2).and_then(|v| v.as_integer());
    let j_opt = l.get_arg(3).and_then(|v| v.as_integer());

    // Collect values while borrowing vm
    let values = {
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            let _ = vm;
            return Err(l.error("bad argument #1 to 'unpack' (table expected)".to_string()));
        };

        let len = table_ref.len();
        let i = i_opt.unwrap_or(1);
        let j = j_opt.unwrap_or(len as i64);

        // Handle empty range
        if i > j {
            return Ok(0);
        }

        // Collect all values
        let mut values = Vec::new();
        for idx in i..=j {
            values.push(table_ref.get_int(idx).unwrap_or(LuaValue::nil()));
        }
        values
    };

    // Push values after vm borrow ends
    let count = values.len();
    for val in values {
        l.push_value(val)?;
    }

    Ok(count)
}

/// table.sort(list [, comp]) - Sort table in place
fn table_sort(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'sort' (table expected)".to_string()))?;
    let comp = l.get_arg(2);

    let Some(table_id) = table_val.as_table_id() else {
        return Err(l.error("bad argument #1 to 'sort' (table expected)".to_string()));
    };

    // Get array length
    let len = {
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(l.error("bad argument #1 to 'sort' (table expected)".to_string()));
        };
        table_ref.len()
    };

    if len <= 1 {
        return Ok(0);
    }

    // For now, we only support default sorting (no custom comparison function)
    // TODO: Implement custom comparison function support when VM call API is available
    if comp.is_some() && !comp.as_ref().map(|v| v.is_nil()).unwrap_or(true) {
        return Err(
            l.error("custom comparison functions in table.sort not yet supported".to_string())
        );
    }

    // Extract values and their string representations (for string sorting)
    let mut values = Vec::with_capacity(len);
    let mut string_cache: std::collections::HashMap<i64, String> = std::collections::HashMap::new();

    {
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(l.error("bad argument #1 to 'sort' (table expected)".to_string()));
        };
        for i in 1..=len as i64 {
            let val = table_ref.get_int(i).unwrap_or(LuaValue::nil());

            // Cache string content if it's a string value
            if let Some(string_id) = val.as_string_id() {
                if let Some(s) = vm.object_pool.get_string(string_id) {
                    string_cache.insert(string_id.index() as i64, s.as_str().to_string());
                }
            }

            values.push(val);
        }
    }

    // Sort using Lua semantics comparison
    values.sort_by(|a, b| lua_compare_values(a, b, &string_cache));

    // Write sorted values back
    {
        let vm = l.vm_mut();
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(l.error("bad argument #1 to 'sort' (table expected)".to_string()));
        };
        for (idx, val) in values.into_iter().enumerate() {
            table_ref.set_int((idx + 1) as i64, val);
        }
    }

    Ok(0)
}

/// Compare two Lua values according to Lua semantics
/// Returns Ordering for sorting purposes
fn lua_compare_values(
    a: &LuaValue,
    b: &LuaValue,
    string_cache: &std::collections::HashMap<i64, String>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    // Both numbers - compare numerically
    if let (Some(n1), Some(n2)) = (a.as_number(), b.as_number()) {
        return n1.partial_cmp(&n2).unwrap_or(Ordering::Equal);
    }

    // Both strings - compare lexicographically
    if a.is_string() && b.is_string() {
        if let (Some(id1), Some(id2)) = (a.as_string_id(), b.as_string_id()) {
            // Try to get from cache
            let s1 = string_cache.get(&(id1.index() as i64));
            let s2 = string_cache.get(&(id2.index() as i64));

            if let (Some(str1), Some(str2)) = (s1, s2) {
                return str1.cmp(str2);
            }

            // Fallback: compare string IDs (maintains stability)
            return id1.index().cmp(&id2.index());
        }
    }

    // Different types or incomparable types
    // Use type ordering for stability (allows mixed-type arrays)
    let type_order_a = lua_type_order(a);
    let type_order_b = lua_type_order(b);
    type_order_a.cmp(&type_order_b)
}

/// Get a type order value for sorting purposes
/// Ensures consistent ordering across different types
fn lua_type_order(val: &LuaValue) -> u8 {
    if val.is_nil() {
        return 0;
    }
    if val.is_boolean() {
        return 1;
    }
    if val.is_number() {
        return 2;
    }
    if val.is_string() {
        return 3;
    }
    if val.is_table() {
        return 4;
    }
    if val.is_function() {
        return 5;
    }
    if val.is_userdata() {
        return 6;
    }
    if val.is_thread() {
        return 7;
    }
    // Unknown type
    255
}
