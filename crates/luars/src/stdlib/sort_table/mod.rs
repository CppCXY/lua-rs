use crate::{LuaResult, LuaValue, lua_value::LuaTableDetail, lua_vm::LuaState};

/// table.sort(list [, comp]) - Sort table in place
pub fn table_sort(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'sort' (table expected)".to_string()))?;
    let comp = l.get_arg(2);

    let Some(table_ref) = table_val.as_table_mut() else {
        return Err(l.error("bad argument #1 to 'sort' (table expected)".to_string()));
    };

    // Get array length
    let len = table_ref.len();
    if len <= 1 {
        return Ok(0);
    }

    // Check if we have a custom comparison function
    let has_comp = comp.is_some() && !comp.as_ref().map(|v| v.is_nil()).unwrap_or(true);
    let comp_func = if has_comp {
        comp.unwrap()
    } else {
        LuaValue::nil()
    };

    let impl_table = &mut table_ref.impl_table;
    let mut sort_arr = match impl_table {
        LuaTableDetail::ValueArray(arr) => arr.array.clone(),
        // LuaTableDetail::TypedArray(arr) => arr
        //     .array
        //     .iter()
        //     .map(|v| LuaValue {
        //         tt: arr.tt,
        //         value: v.clone(),
        //     })
        //     .collect(),
        LuaTableDetail::HashTable(_) => {
            // Sort HashTable
            return Err(l.error("bad argument #1 to 'sort' (array expected)".to_string()));
        }
    };

    // // Sort using Lua semantics comparison
    if has_comp {
        // Custom comparison function
        sort_arr.sort_by(|a, b| {
            // Call comp(a, b)
            l.push_value(comp_func).ok();
            l.push_value(*a).ok();
            l.push_value(*b).ok();

            let func_idx = l.get_top() - 3;
            let result = l.pcall_stack_based(func_idx, 2);

            match result {
                Ok((true, result_count)) => {
                    // Get the result
                    let cmp_result = if result_count > 0 {
                        l.stack_get(func_idx).unwrap_or(LuaValue::nil())
                    } else {
                        LuaValue::nil()
                    };

                    // Clean up stack
                    l.set_top(func_idx);

                    // Convert to bool: nil and false are false, everything else is true
                    let is_less = if cmp_result.is_nil() {
                        false
                    } else if let Some(b) = cmp_result.as_boolean() {
                        b
                    } else {
                        true
                    };

                    if is_less {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                }
                _ => {
                    // Error or false - treat as not less than
                    l.set_top(func_idx);
                    std::cmp::Ordering::Equal
                }
            }
        });
    } else {
        // Default comparison
        sort_arr.sort_by(|a, b| lua_compare_values(a, b));
    }

    // Write back sorted array
    let Some(table_ref) = table_val.as_table_mut() else {
        return Err(l.error("bad argument #1 to 'sort' (table expected)".to_string()));
    };

    match &mut table_ref.impl_table {
        LuaTableDetail::ValueArray(arr) => {
            arr.array = sort_arr;
        }
        // LuaTableDetail::TypedArray(arr) => {
        //     arr.array = sort_arr.into_iter().map(|v| v.value).collect();
        // }
        LuaTableDetail::HashTable(_) => {
            // Sort HashTable
            return Err(l.error("bad argument #1 to 'sort' (array expected)".to_string()));
        }
    };

    // table_ref.array = arr;
    Ok(0)
}

/// Compare two Lua values according to Lua semantics
/// Returns Ordering for sorting purposes
fn lua_compare_values(a: &LuaValue, b: &LuaValue) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    // Both numbers - compare numerically
    if let (Some(n1), Some(n2)) = (a.as_number(), b.as_number()) {
        return n1.partial_cmp(&n2).unwrap_or(Ordering::Equal);
    }

    // Both strings - compare lexicographically
    if a.is_string() && b.is_string() {
        if let (Some(str1), Some(str2)) = (a.as_str(), b.as_str()) {
            return str1.cmp(str2);
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
