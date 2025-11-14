// Table library
// Implements: concat, insert, move, pack, remove, sort, unpack

use crate::lib_registry::{LibraryModule, get_arg, require_arg};
use crate::lua_value::{LuaValue, MultiValue};
use crate::vm::VM;

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
fn table_concat(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "table.concat")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'table.concat' (table expected)".to_string())?;

    let sep = get_arg(vm, 1)
        .and_then(|v| v.as_string())
        .map(|s| s.as_str().to_string())
        .unwrap_or_default();

    let table_ref = table.borrow();
    let len = table_ref.len();

    let i = get_arg(vm, 2).and_then(|v| v.as_integer()).unwrap_or(1);

    let j = get_arg(vm, 3)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    let mut parts = Vec::new();
    for idx in i..=j {
        let key = LuaValue::Integer(idx);
        let value = table_ref.raw_get(&key).unwrap_or(LuaValue::Nil);

        if let Some(s) = value.as_string() {
            parts.push(s.as_str().to_string());
        } else {
            return Err(format!(
                "bad value at index {} in 'table.concat' (string expected)",
                idx
            ));
        }
    }

    let result = vm.create_string(parts.join(&sep));
    Ok(MultiValue::single(LuaValue::String(result)))
}

/// table.insert(list, [pos,] value) - Insert element
fn table_insert(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "table.insert")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'table.insert' (table expected)".to_string())?;

    let argc = crate::lib_registry::arg_count(vm);

    let mut table_ref = table.borrow_mut();
    let len = table_ref.len();

    if argc == 2 {
        // table.insert(list, value)
        let value = require_arg(vm, 1, "table.insert")?;
        table_ref.raw_set(LuaValue::Integer(len as i64 + 1), value);
    } else if argc == 3 {
        // table.insert(list, pos, value)
        let pos = require_arg(vm, 1, "table.insert")?
            .as_integer()
            .ok_or_else(|| "bad argument #2 to 'table.insert' (number expected)".to_string())?;

        let value = require_arg(vm, 2, "table.insert")?;

        if pos < 1 || pos > len as i64 + 1 {
            return Err(format!(
                "bad argument #2 to 'table.insert' (position out of bounds)"
            ));
        }

        table_ref.insert_array_at(pos as usize - 1, value)?;
    } else {
        return Err("wrong number of arguments to 'table.insert'".to_string());
    }

    Ok(MultiValue::empty())
}

/// table.remove(list [, pos]) - Remove element
fn table_remove(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "table.remove")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'table.remove' (table expected)".to_string())?;

    let mut table_ref = table.borrow_mut();
    let len = table_ref.len();

    if len == 0 {
        return Ok(MultiValue::single(LuaValue::Nil));
    }

    let pos = get_arg(vm, 1)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    if pos < 1 || pos > len as i64 {
        return Err(format!(
            "bad argument #2 to 'table.remove' (position out of bounds)"
        ));
    }

    let removed = table_ref.remove_array_at(pos as usize - 1)?;
    Ok(MultiValue::single(removed))
}

/// table.move(a1, f, e, t [, a2]) - Move elements
fn table_move(vm: &mut VM) -> Result<MultiValue, String> {
    let src = require_arg(vm, 0, "table.move")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'table.move' (table expected)".to_string())?;

    let f = require_arg(vm, 1, "table.move")?
        .as_integer()
        .ok_or_else(|| "bad argument #2 to 'table.move' (number expected)".to_string())?;

    let e = require_arg(vm, 2, "table.move")?
        .as_integer()
        .ok_or_else(|| "bad argument #3 to 'table.move' (number expected)".to_string())?;

    let t = require_arg(vm, 3, "table.move")?
        .as_integer()
        .ok_or_else(|| "bad argument #4 to 'table.move' (number expected)".to_string())?;

    let dst = get_arg(vm, 4)
        .and_then(|v| v.as_table())
        .unwrap_or_else(|| src.clone());

    // Copy elements
    let mut values = Vec::new();
    {
        let src_ref = src.borrow();
        for i in f..=e {
            let val = src_ref
                .raw_get(&LuaValue::Integer(i))
                .unwrap_or(LuaValue::Nil);
            values.push(val);
        }
    }

    {
        let mut dst_ref = dst.borrow_mut();
        for (offset, val) in values.into_iter().enumerate() {
            dst_ref.raw_set(LuaValue::Integer(t + offset as i64), val);
        }
    }

    Ok(MultiValue::single(LuaValue::Table(dst)))
}

/// table.pack(...) - Pack values into table
fn table_pack(vm: &mut VM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);

    let table = vm.create_table();
    let mut table_ref = table.borrow_mut();

    for (i, arg) in args.iter().enumerate() {
        table_ref.raw_set(LuaValue::Integer(i as i64 + 1), arg.clone());
    }

    // Set 'n' field
    let n_key = vm.create_string("n".to_string());
    table_ref.raw_set(
        LuaValue::String(n_key),
        LuaValue::Integer(args.len() as i64),
    );

    drop(table_ref);
    Ok(MultiValue::single(LuaValue::Table(table)))
}

/// table.unpack(list [, i [, j]]) - Unpack table into values
fn table_unpack(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "table.unpack")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'table.unpack' (table expected)".to_string())?;

    let table_ref = table.borrow();
    let len = table_ref.len();

    let i = get_arg(vm, 1).and_then(|v| v.as_integer()).unwrap_or(1);

    let j = get_arg(vm, 2)
        .and_then(|v| v.as_integer())
        .unwrap_or(len as i64);

    let mut result = Vec::new();
    for idx in i..=j {
        let val = table_ref
            .raw_get(&LuaValue::Integer(idx))
            .unwrap_or(LuaValue::Nil);
        result.push(val);
    }

    Ok(MultiValue::multiple(result))
}

/// table.sort(list [, comp]) - Sort table in place
fn table_sort(vm: &mut VM) -> Result<MultiValue, String> {
    let table = require_arg(vm, 0, "table.sort")?
        .as_table()
        .ok_or_else(|| "bad argument #1 to 'table.sort' (table expected)".to_string())?;

    let comp = get_arg(vm, 1);

    let mut table_ref = table.borrow_mut();

    // Extract sortable values
    let values = table_ref
        .get_array_part()
        .ok_or("table.sort: table does not have an array part")?;

    let len = values.len();
    if len <= 1 {
        return Ok(MultiValue::empty());
    }

    // If no comparison function, use default ordering
    if comp.is_none() || comp.as_ref().map(|v| v.is_nil()).unwrap_or(true) {
        values.sort();
        return Ok(MultiValue::empty());
    }

    let comp_func = comp.unwrap();

    // Clone values for sorting (we need to modify the table during comparison calls)
    let mut sorted_values: Vec<LuaValue> = values.iter().cloned().collect();

    // Drop the mutable borrow before we start calling the comparison function
    drop(table_ref);

    // Custom comparison using Lua function
    // We need to use a comparison that calls the Lua function
    // Since we can't borrow vm mutably during sort, we'll do a simple insertion sort
    for i in 1..sorted_values.len() {
        let mut j = i;
        while j > 0 {
            // Call comparison function: comp(a, b) should return true if a < b
            let args = vec![sorted_values[j].clone(), sorted_values[j - 1].clone()];
            let result = match vm.call_metamethod(&comp_func, &args) {
                Ok(Some(val)) => val.is_truthy(),
                Ok(None) => false,
                Err(e) => return Err(format!("error in sort comparison function: {}", e)),
            };

            if result {
                sorted_values.swap(j, j - 1);
                j -= 1;
            } else {
                break;
            }
        }
    }

    // Write back the sorted values
    let mut table_ref = table.borrow_mut();
    if let Some(array) = table_ref.get_array_part() {
        for (i, val) in sorted_values.into_iter().enumerate() {
            array[i] = val;
        }
    }

    Ok(MultiValue::empty())
}
