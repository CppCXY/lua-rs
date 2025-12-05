// Table library
// Implements: concat, insert, move, pack, remove, sort, unpack

use crate::lib_registry::{LibraryModule, arg_count, get_arg, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

pub fn create_table_lib() -> LibraryModule {
    crate::lib_module!("table", {
        "concat" => table_concat,
        "insert" => table_insert,
        "move" => table_move,
        "pack" => table_pack,
        "remove" => table_remove,
        "sort" => table_sort,
        "unpack" => table_unpack,
    })
}

/// table.concat(list [, sep [, i [, j]]]) - Concatenate table elements
fn table_concat(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 1, "table.concat")?;

    let sep_value = get_arg(vm, 2);
    let sep = match sep_value {
        Some(v) => {
            if let Some(string_id) = v.as_string_id() {
                if let Some(s) = vm.object_pool.get_string(string_id) {
                    s.as_str().to_string()
                } else {
                    return Err(
                        vm.error("bad argument #2 to 'table.concat' (string expected)".to_string())
                    );
                }
            } else {
                return Err(
                    vm.error("bad argument #2 to 'table.concat' (string expected)".to_string())
                );
            }
        }
        None => "".to_string(),
    };

    let Some(table_id) = table_val.as_table_id() else {
        return Err(vm.error("Invalid table".to_string()));
    };
    let i = get_arg(vm, 3).and_then(|v| v.as_integer()).unwrap_or(1);

    let (len, parts) = {
        let Some(table_borrowed) = vm.object_pool.get_table(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        let len = table_borrowed.len();
        let j = get_arg(vm, 4)
            .and_then(|v| v.as_integer())
            .unwrap_or(len as i64);

        if table_borrowed.array.is_empty() {
            (len, Vec::new())
        } else {
            let mut parts = Vec::new();
            for idx in i..=j {
                if let Some(value) = table_borrowed.array.get(idx as usize - 1) {
                    if let Some(string_id) = value.as_string_id() {
                        if let Some(s) = vm.object_pool.get_string(string_id) {
                            parts.push(s.as_str().to_string());
                        } else {
                            return Err(vm.error(format!(
                                "bad value at index {} in 'table.concat' (string expected)",
                                idx
                            )));
                        }
                    } else {
                        return Err(vm.error(format!(
                            "bad value at index {} in 'table.concat' (string expected)",
                            idx
                        )));
                    }
                }
            }
            (len, parts)
        }
    };

    if parts.is_empty() && len == 0 {
        let s: LuaValue = vm.create_string("");
        return Ok(MultiValue::single(s));
    }

    let result = vm.create_string(&parts.join(&sep));
    Ok(MultiValue::single(result))
}

/// table.insert(list, [pos,] value) - Insert element
fn table_insert(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 1, "table.insert")?;
    let argc = arg_count(vm);

    let Some(table_id) = table_val.as_table_id() else {
        return Err(vm.error("Invalid table".to_string()));
    };

    let len = {
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        table_ref.len()
    };

    if argc == 2 {
        // table.insert(list, value) - append at end
        let value = require_arg(vm, 2, "table.insert")?;
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        match table_ref.insert_array_at(len, value) {
            Ok(_) => {}
            Err(e) => {
                return Err(vm.error(format!("error inserting into table: {}", e)));
            }
        }
    } else if argc == 3 {
        // table.insert(list, pos, value)
        let pos = require_arg(vm, 2, "table.insert")?
            .as_integer()
            .ok_or_else(|| {
                vm.error("bad argument #2 to 'table.insert' (number expected)".to_string())
            })?;

        let value = require_arg(vm, 3, "table.insert")?;

        if pos < 1 || pos > len as i64 + 1 {
            return Err(vm.error(format!(
                "bad argument #2 to 'table.insert' (position out of bounds)"
            )));
        }
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        match table_ref.insert_array_at(pos as usize - 1, value) {
            Ok(_) => {}
            Err(e) => {
                return Err(vm.error(format!("error inserting into table: {}", e)));
            }
        }
    } else {
        return Err(vm.error("wrong number of arguments to 'table.insert'".to_string()));
    }

    Ok(MultiValue::empty())
}

/// table.remove(list [, pos]) - Remove element
fn table_remove(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 1, "table.remove")?;

    let Some(table_id) = table_val.as_table_id() else {
        return Err(vm.error("Invalid table".to_string()));
    };

    let len = {
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        table_ref.len()
    };

    if len == 0 {
        return Ok(MultiValue::single(LuaValue::nil()));
    }

    let pos = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    if pos < 1 || pos > len as i64 {
        return Err(vm.error(format!(
            "bad argument #2 to 'table.remove' (position out of bounds)"
        )));
    }

    let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
        return Err(vm.error("Invalid table".to_string()));
    };
    let removed = match table_ref.remove_array_at(pos as usize - 1) {
        Ok(val) => val,
        Err(e) => return Err(vm.error(format!("error removing from table: {}", e))),
    };
    Ok(MultiValue::single(removed))
}

/// table.move(a1, f, e, t [, a2]) - Move elements
fn table_move(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let src_val = require_arg(vm, 1, "table.move")?;

    let f = require_arg(vm, 2, "table.move")?
        .as_integer()
        .ok_or_else(|| vm.error("bad argument #2 to 'table.move' (number expected)".to_string()))?;

    let e = require_arg(vm, 3, "table.move")?
        .as_integer()
        .ok_or_else(|| vm.error("bad argument #3 to 'table.move' (number expected)".to_string()))?;
    let t = require_arg(vm, 4, "table.move")?
        .as_integer()
        .ok_or_else(|| vm.error("bad argument #4 to 'table.move' (number expected)".to_string()))?;

    let dst_value = get_arg(vm, 5).unwrap_or_else(|| src_val.clone());
    // Copy elements
    let mut values = Vec::new();
    {
        let Some(src_id) = src_val.as_table_id() else {
            return Err(vm.error("Invalid source table".to_string()));
        };
        let Some(src_ref) = vm.object_pool.get_table(src_id) else {
            return Err(vm.error("Invalid source table".to_string()));
        };

        for i in f..=e {
            let val = src_ref.get_int(i).unwrap_or(LuaValue::nil());
            values.push(val);
        }
    }

    {
        let Some(dst_id) = dst_value.as_table_id() else {
            return Err(vm.error("Invalid destination table".to_string()));
        };
        let Some(dst_ref) = vm.object_pool.get_table_mut(dst_id) else {
            return Err(vm.error("Invalid destination table".to_string()));
        };
        for (offset, val) in values.into_iter().enumerate() {
            dst_ref.set_int(t + offset as i64, val);
        }
    }

    Ok(MultiValue::single(dst_value))
}

/// table.pack(...) - Pack values into table
fn table_pack(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = crate::lib_registry::get_args(vm);
    let table = vm.create_table(args.len(), 1);
    // Set 'n' field
    let n_key = vm.create_string("n");
    let Some(table_id) = table.as_table_id() else {
        return Err(vm.error("Invalid table".to_string()));
    };
    let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
        return Err(vm.error("Invalid table".to_string()));
    };

    for (i, arg) in args.iter().enumerate() {
        table_ref.set_int(i as i64 + 1, arg.clone());
    }

    table_ref.raw_set(n_key, LuaValue::integer(args.len() as i64));
    Ok(MultiValue::single(table))
}

/// table.unpack(list [, i [, j]]) - Unpack table into values
/// OPTIMIZED: Pre-allocate Vec and use direct array access when possible
fn table_unpack(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr as usize;
    let top = frame.top as usize;
    
    if top <= 1 {
        return Err(vm.error("bad argument #1 to 'unpack' (table expected)".to_string()));
    }
    
    let table_val = vm.register_stack[base_ptr + 1];
    let Some(table_id) = table_val.as_table_id() else {
        return Err(vm.error("bad argument #1 to 'unpack' (table expected)".to_string()));
    };
    let Some(table_ref) = vm.object_pool.get_table(table_id) else {
        return Err(vm.error("Invalid table".to_string()));
    };
    
    let len = table_ref.len();
    
    // Get i (default 1) - direct stack access
    let i = if top > 2 {
        vm.register_stack[base_ptr + 2].as_integer().unwrap_or(1)
    } else {
        1
    };
    
    // Get j (default len) - direct stack access
    let j = if top > 3 {
        vm.register_stack[base_ptr + 3].as_integer().unwrap_or(len as i64)
    } else {
        len as i64
    };
    
    // Handle empty range
    if i > j {
        return Ok(MultiValue::empty());
    }
    
    let count = (j - i + 1) as usize;
    
    // FAST PATH: Small unpacks (common case)
    if count == 1 {
        let val = table_ref.get_int(i).unwrap_or(LuaValue::nil());
        return Ok(MultiValue::single(val));
    }
    
    // Pre-allocate with exact capacity
    let mut result = Vec::with_capacity(count);
    
    // FAST PATH: If unpacking from beginning and array part is large enough
    // we can copy directly from array
    if i == 1 && table_ref.array.len() >= count {
        for idx in 0..count {
            result.push(table_ref.array[idx]);
        }
    } else {
        // General case: use get_int
        for idx in i..=j {
            let val = table_ref.get_int(idx).unwrap_or(LuaValue::nil());
            result.push(val);
        }
    }

    Ok(MultiValue::multiple(result))
}

/// table.sort(list [, comp]) - Sort table in place
fn table_sort(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 1, "table.sort")?;
    let comp = get_arg(vm, 2);

    let Some(table_id) = table_val.as_table_id() else {
        return Err(vm.error("Invalid table".to_string()));
    };

    // Get array length
    let len = {
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        table_ref.len()
    };

    if len <= 1 {
        return Ok(MultiValue::empty());
    }

    // Check if array is empty
    let is_empty = {
        let Some(table_ref) = vm.object_pool.get_table(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        table_ref.array.is_empty()
    };

    if is_empty {
        return Ok(MultiValue::empty());
    }

    // If no comparison function, use default ordering
    if comp.is_none() || comp.as_ref().map(|v| v.is_nil()).unwrap_or(true) {
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        table_ref.array.sort();
        return Ok(MultiValue::empty());
    }

    let comp_func = comp.unwrap();

    // For custom comparison, we need to extract array, sort it externally, then put it back
    let mut arr = {
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        std::mem::take(&mut table_ref.array)
    };

    // Custom comparison using VM's call function
    arr.sort_by(|a, b| {
        // Call comparison function: comp(a, b) should return true if a < b
        let args = vec![a.clone(), b.clone()];
        match vm.call_function_internal(comp_func.clone(), args) {
            Ok(results) => {
                let result = results.get(0).map(|v| v.is_truthy()).unwrap_or(false);
                if result {
                    std::cmp::Ordering::Less
                } else {
                    // Need to check comp(b, a) to distinguish Equal from Greater
                    let args_rev = vec![b.clone(), a.clone()];
                    match vm.call_function_internal(comp_func.clone(), args_rev) {
                        Ok(results_rev) => {
                            let result_rev =
                                results_rev.get(0).map(|v| v.is_truthy()).unwrap_or(false);
                            if result_rev {
                                std::cmp::Ordering::Greater
                            } else {
                                std::cmp::Ordering::Equal
                            }
                        }
                        Err(_) => std::cmp::Ordering::Equal, // On error, treat as equal
                    }
                }
            }
            Err(_) => std::cmp::Ordering::Equal, // On error, treat as equal
        }
    });

    // Put the sorted array back
    {
        let Some(table_ref) = vm.object_pool.get_table_mut(table_id) else {
            return Err(vm.error("Invalid table".to_string()));
        };
        table_ref.array = arr;
    }

    Ok(MultiValue::empty())
}
