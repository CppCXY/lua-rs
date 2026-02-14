use crate::{LuaResult, LuaValue, lua_vm::LuaState};

/// table.sort(list [, comp]) - Sort table in place
/// Optimized: extracts array to Vec, sorts, writes back using raw API
pub fn table_sort(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| l.error("bad argument #1 to 'sort' (table expected)".to_string()))?;
    let comp = l.get_arg(2);

    let table = table_val
        .as_table()
        .ok_or_else(|| l.error("bad argument #1 to 'sort' (table expected)".to_string()))?;

    let len = table.len();
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

    // Extract array elements into a Vec using raw access
    let mut sort_arr = Vec::with_capacity(len);
    for i in 1..=len as i64 {
        let val = table.raw_geti(i).unwrap_or(LuaValue::nil());
        sort_arr.push(val);
    }

    // Sort
    if has_comp {
        // Custom comparison function
        sort_arr.sort_by(|a, b| {
            l.push_value(comp_func).ok();
            l.push_value(*a).ok();
            l.push_value(*b).ok();

            let func_idx = l.get_top() - 3;
            let result = l.pcall_stack_based(func_idx, 2);

            match result {
                Ok((true, result_count)) => {
                    let cmp_result = if result_count > 0 {
                        l.stack_get(func_idx).unwrap_or(LuaValue::nil())
                    } else {
                        LuaValue::nil()
                    };
                    let _ = l.set_top(func_idx);
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
                    let _ = l.set_top(func_idx);
                    std::cmp::Ordering::Equal
                }
            }
        });
    } else {
        // Default comparison
        sort_arr.sort_by(|a, b| lua_compare_values(a, b));
    }

    // Write back sorted array using raw API
    let table = table_val.as_table_mut().unwrap();
    for (i, val) in sort_arr.into_iter().enumerate() {
        table.raw_seti((i + 1) as i64, val);
    }

    // GC write barrier: table was modified directly (sorted elements may be collectable)
    if let Some(gc_ptr) = table_val.as_gc_ptr() {
        l.gc_barrier_back(gc_ptr);
    }

    Ok(0)
}

/// Compare two Lua values according to Lua semantics
fn lua_compare_values(a: &LuaValue, b: &LuaValue) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    if let (Some(n1), Some(n2)) = (a.as_number(), b.as_number()) {
        return n1.partial_cmp(&n2).unwrap_or(Ordering::Equal);
    }

    if a.is_string() && b.is_string() {
        if let (Some(str1), Some(str2)) = (a.as_str(), b.as_str()) {
            return str1.cmp(str2);
        }
    }

    let type_order_a = lua_type_order(a);
    let type_order_b = lua_type_order(b);
    type_order_a.cmp(&type_order_b)
}

fn lua_type_order(val: &LuaValue) -> u8 {
    if val.is_nil() { return 0; }
    if val.is_boolean() { return 1; }
    if val.is_number() { return 2; }
    if val.is_string() { return 3; }
    if val.is_table() { return 4; }
    if val.is_function() { return 5; }
    if val.is_userdata() { return 6; }
    if val.is_thread() { return 7; }
    255
}
