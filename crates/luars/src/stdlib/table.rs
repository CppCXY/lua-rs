// Table library
// Implements: concat, insert, move, pack, remove, sort, unpack

use crate::lib_registry::{LibraryModule, get_arg, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaError, LuaResult, LuaVM};

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
    let table_val = require_arg(vm, 0, "table.concat")?;

    let sep_value = get_arg(vm, 1);
    let sep = match sep_value {
        Some(v) => vm
            .get_string(&v)
            .ok_or_else(|| {
                vm.error(
                    "bad argument #2 to 'table.concat' (string expected)".to_string(),
                )
            })?
            .as_str()
            .to_string(),
        None => "".to_string(),
    };

    let table_ptr = table_val
        .as_table_ptr()
        .ok_or(vm.error("Invalid table".to_string()))?;
    let len = unsafe { (*table_ptr).borrow().len() };

    let i = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(1);

    let j = get_arg(vm, 3)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    let mut parts = Vec::new();
    for idx in i..=j {
        let key = LuaValue::integer(idx);
        let value = unsafe {
            (*table_ptr)
                .borrow()
                .raw_get(&key)
                .unwrap_or(LuaValue::nil())
        };

        unsafe {
            if let Some(s) = value.as_string() {
                parts.push(s.as_str().to_string());
            } else {
                return Err(vm.error(format!(
                    "bad value at index {} in 'table.concat' (string expected)",
                    idx
                )));
            }
        }
    }

    let result = vm.create_string(&parts.join(&sep));
    Ok(MultiValue::single(result))
}

/// table.insert(list, [pos,] value) - Insert element
fn table_insert(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 0, "table.insert")?;
    let argc = crate::lib_registry::arg_count(vm);

    // CRITICAL: Use direct pointer to avoid HashMap lookup per insert!
    let table_ptr = table_val
        .as_table_ptr()
        .ok_or(vm.error("Invalid table".to_string()))?;

    let len = unsafe { (*table_ptr).borrow().len() };

    if argc == 2 {
        // table.insert(list, value) - append at end
        let value = require_arg(vm, 1, "table.insert")?;
        unsafe {
            (*table_ptr)
                .borrow_mut()
                .raw_set(LuaValue::integer(len as i64 + 1), value)
        };
    } else if argc == 3 {
        // table.insert(list, pos, value)
        let pos = require_arg(vm, 1, "table.insert")?
            .as_integer()
            .ok_or_else(|| {
                vm.error(
                    "bad argument #2 to 'table.insert' (number expected)".to_string(),
                )
            })?;

        let value = require_arg(vm, 2, "table.insert")?;

        if pos < 1 || pos > len as i64 + 1 {
            return Err(vm.error(format!(
                "bad argument #2 to 'table.insert' (position out of bounds)"
            )));
        }

        unsafe {
            (*table_ptr)
                .borrow_mut()
                .insert_array_at(pos as usize - 1, value)?
        };
    } else {
        return Err(vm.error(
            "wrong number of arguments to 'table.insert'".to_string(),
        ));
    }

    Ok(MultiValue::empty())
}

/// table.remove(list [, pos]) - Remove element
fn table_remove(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 0, "table.remove")?;

    // CRITICAL: Use direct pointer to avoid HashMap lookup
    let table_ptr = table_val
        .as_table_ptr()
        .ok_or(vm.error("Invalid table".to_string()))?;

    let len = unsafe { (*table_ptr).borrow().len() };

    if len == 0 {
        return Ok(MultiValue::single(LuaValue::nil()));
    }

    let pos = get_arg(vm, 1)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    if pos < 1 || pos > len as i64 {
        return Err(vm.error(format!(
            "bad argument #2 to 'table.remove' (position out of bounds)"
        )));
    }

    let removed = unsafe {
        (*table_ptr)
            .borrow_mut()
            .remove_array_at(pos as usize - 1)?
    };
    Ok(MultiValue::single(removed))
}

/// table.move(a1, f, e, t [, a2]) - Move elements
fn table_move(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let src_val = require_arg(vm, 0, "table.move")?;

    let f = require_arg(vm, 1, "table.move")?
        .as_integer()
        .ok_or_else(|| {
            vm.error("bad argument #2 to 'table.move' (number expected)".to_string())
        })?;

    let e = require_arg(vm, 2, "table.move")?
        .as_integer()
        .ok_or_else(|| {
            vm.error("bad argument #3 to 'table.move' (number expected)".to_string())
        })?;
    let t = require_arg(vm, 3, "table.move")?
        .as_integer()
        .ok_or_else(|| {
            vm.error("bad argument #4 to 'table.move' (number expected)".to_string())
        })?;

    let dst_value = get_arg(vm, 4).unwrap_or_else(|| src_val.clone());

    // Copy elements
    let mut values = Vec::new();
    {
        let src_ptr = src_val
            .as_table_ptr()
            .ok_or(vm.error("Invalid source table".to_string()))?;
        let src_ref = unsafe { (*src_ptr).borrow() };
        for i in f..=e {
            let val = src_ref
                .raw_get(&LuaValue::integer(i))
                .unwrap_or(LuaValue::nil());
            values.push(val);
        }
    }

    {
        let dst_ptr = dst_value.as_table_ptr().ok_or(vm.error(
            "Invalid destination table".to_string(),
        ))?;
        let mut dst_ref = unsafe { (*dst_ptr).borrow_mut() };
        for (offset, val) in values.into_iter().enumerate() {
            dst_ref.raw_set(LuaValue::integer(t + offset as i64), val);
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
    let table_ptr = table
        .as_table_ptr()
        .ok_or(vm.error("Invalid table".to_string()))?;
    let mut table_ref = unsafe { (*table_ptr).borrow_mut() };

    for (i, arg) in args.iter().enumerate() {
        table_ref.raw_set(LuaValue::integer(i as i64 + 1), arg.clone());
    }

    table_ref.raw_set(n_key, LuaValue::integer(args.len() as i64));

    drop(table_ref);
    Ok(MultiValue::single(table))
}

/// table.unpack(list [, i [, j]]) - Unpack table into values
fn table_unpack(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 0, "table.unpack")?;
    let table_ptr = table_val
        .as_table_ptr()
        .ok_or(vm.error("Invalid table".to_string()))?;
    let table_ref = unsafe { (*table_ptr).borrow() };
    let len = table_ref.len();

    let i = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(1);

    let j = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    let mut result = Vec::new();
    for idx in i..=j {
        let val = table_ref
            .raw_get(&LuaValue::integer(idx))
            .unwrap_or(LuaValue::nil());
        result.push(val);
    }

    Ok(MultiValue::multiple(result))
}

/// table.sort(list [, comp]) - Sort table in place
fn table_sort(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let table_val = require_arg(vm, 0, "table.sort")?;
    let comp = get_arg(vm, 1);

    // Get array length and extract values
    let len = {
        let table_ptr = table_val
            .as_table_ptr()
            .ok_or(vm.error("Invalid table".to_string()))?;
        let table_ref = unsafe { (*table_ptr).borrow() };
        table_ref.len()
    };

    if len <= 1 {
        return Ok(MultiValue::empty());
    }

    // Extract values from array part [1..len]
    let mut values = Vec::with_capacity(len);
    {
        let table_ptr = table_val
            .as_table_ptr()
            .ok_or(vm.error("Invalid table".to_string()))?;
        let table_ref = unsafe { (*table_ptr).borrow() };
        for i in 1..=len {
            if let Some(val) = table_ref.get_int(i as i64) {
                values.push(val);
            } else {
                values.push(LuaValue::nil());
            }
        }
    }

    // If no comparison function, use default ordering
    if comp.is_none() || comp.as_ref().map(|v| v.is_nil()).unwrap_or(true) {
        values.sort();
        // Write back sorted values
        let table_ptr = table_val
            .as_table_ptr()
            .ok_or(vm.error("Invalid table".to_string()))?;
        let mut table_ref = unsafe { (*table_ptr).borrow_mut() };
        for (i, val) in values.into_iter().enumerate() {
            table_ref.set_int((i + 1) as i64, val);
        }
        return Ok(MultiValue::empty());
    }

    let comp_func = comp.unwrap();

    // Custom comparison using Lua function - use sort_by with comparator
    // We need to use sort_by_cached_key or manual sort_by to handle comparison errors
    values.sort_by(|a, b| {
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

    // Write back the sorted values
    let table_ref_cell = vm
        .get_table(&table_val)
        .ok_or(vm.error("Invalid table".to_string()))?;
    let mut table_ref = table_ref_cell.borrow_mut();
    for (i, val) in values.into_iter().enumerate() {
        table_ref.set_int((i + 1) as i64, val);
    }

    Ok(MultiValue::empty())
}
