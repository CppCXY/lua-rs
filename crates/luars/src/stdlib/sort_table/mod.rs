use crate::{LuaResult, LuaValue, lua_vm::LuaState};

/// table.sort(list [, comp]) - Sort table in place
/// Uses manual sort algorithm to properly propagate Lua errors/yields.
pub fn table_sort(l: &mut LuaState) -> LuaResult<usize> {
    let table_val = l
        .get_arg(1)
        .ok_or_else(|| crate::stdlib::debug::argerror(l, 1, "table expected"))?;
    let comp = l.get_arg(2);

    if !table_val.is_table() {
        return Err(crate::stdlib::debug::arg_typeerror(
            l, 1, "table", &table_val,
        ));
    }

    // Use obj_len to respect __len metamethod (like C Lua's aux_getn / luaL_len)
    let len = l.obj_len(&table_val)?;

    // C Lua: luaL_argcheck(L, n < INT_MAX, 1, "array too big");
    if len >= i32::MAX as i64 {
        return Err(l.error("bad argument #1 to 'sort' (array too big)".to_string()));
    }

    if len <= 1 {
        return Ok(0);
    }

    let has_comp = comp.is_some() && !comp.as_ref().map(|v| v.is_nil()).unwrap_or(true);
    let comp_func = if has_comp {
        comp.unwrap()
    } else {
        LuaValue::nil()
    };

    // Block yields during sort — sort is a non-continuable C call boundary
    l.nny += 1;

    // Sort in-place on the Lua table using an insertion sort + quicksort
    // that calls comparison through unprotected Lua calls
    let result = sort_range(l, &table_val, &comp_func, has_comp, 1, len as i64);

    // Restore yieldability before returning (even on error)
    l.nny -= 1;

    result?;

    // GC write barrier
    if let Some(gc_ptr) = table_val.as_gc_ptr() {
        l.gc_barrier_back(gc_ptr);
    }

    Ok(0)
}

/// Compare two values using the comparison function or default < operator.
fn sort_compare(
    l: &mut LuaState,
    a: LuaValue,
    b: LuaValue,
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<bool> {
    if has_comp {
        let results = l.call(*comp_func, vec![a, b])?;
        Ok(results.first().map(|v| v.is_truthy()).unwrap_or(false))
    } else {
        // Default comparison: use < operator
        default_less_than(l, &a, &b)
    }
}

/// Default less-than comparison (like Lua's < operator with metamethods).
fn default_less_than(l: &mut LuaState, a: &LuaValue, b: &LuaValue) -> LuaResult<bool> {
    l.obj_lt(a, b)
}

/// Sort a range [lo, hi] of the table in place.
/// Uses insertion sort for small ranges, quicksort for larger.
fn sort_range(
    l: &mut LuaState,
    table: &LuaValue,
    comp_func: &LuaValue,
    has_comp: bool,
    lo: i64,
    hi: i64,
) -> LuaResult<()> {
    if lo >= hi {
        return Ok(());
    }
    let n = hi - lo + 1;
    if n <= 3 {
        // Small range: use insertion sort (no invalid order detection needed for <=3)
        for i in (lo + 1)..=hi {
            let t = table.as_table().unwrap();
            let key = t.raw_geti(i).unwrap_or(LuaValue::nil());
            let mut j = i - 1;
            loop {
                let t = table.as_table().unwrap();
                let val_j = t.raw_geti(j).unwrap_or(LuaValue::nil());
                if sort_compare(l, key, val_j, comp_func, has_comp)? {
                    let t = table.as_table_mut().unwrap();
                    t.raw_seti(j + 1, val_j);
                    if j <= lo {
                        let t = table.as_table_mut().unwrap();
                        t.raw_seti(lo, key);
                        break;
                    }
                    j -= 1;
                } else {
                    let t = table.as_table_mut().unwrap();
                    t.raw_seti(j + 1, key);
                    break;
                }
            }
        }
        return Ok(());
    }

    // Quicksort with median-of-3 pivot
    let mid = lo + (hi - lo) / 2;
    // Sort lo, mid, hi
    {
        let t = table.as_table().unwrap();
        let v_lo = t.raw_geti(lo).unwrap_or(LuaValue::nil());
        let v_mid = t.raw_geti(mid).unwrap_or(LuaValue::nil());
        if sort_compare(l, v_mid, v_lo, comp_func, has_comp)? {
            let t = table.as_table_mut().unwrap();
            t.raw_seti(lo, v_mid);
            t.raw_seti(mid, v_lo);
        }
    }
    {
        let t = table.as_table().unwrap();
        let v_mid = t.raw_geti(mid).unwrap_or(LuaValue::nil());
        let v_hi = t.raw_geti(hi).unwrap_or(LuaValue::nil());
        if sort_compare(l, v_hi, v_mid, comp_func, has_comp)? {
            let t = table.as_table_mut().unwrap();
            t.raw_seti(mid, v_hi);
            t.raw_seti(hi, v_mid);
            let t = table.as_table().unwrap();
            let v_lo = t.raw_geti(lo).unwrap_or(LuaValue::nil());
            let v_mid = t.raw_geti(mid).unwrap_or(LuaValue::nil());
            if sort_compare(l, v_mid, v_lo, comp_func, has_comp)? {
                let t = table.as_table_mut().unwrap();
                t.raw_seti(lo, v_mid);
                t.raw_seti(mid, v_lo);
            }
        }
    }
    if n <= 3 {
        return Ok(());
    }

    // Pivot is now at mid
    let t = table.as_table().unwrap();
    let pivot = t.raw_geti(mid).unwrap_or(LuaValue::nil());

    // Move pivot to hi-1
    let t = table.as_table().unwrap();
    let v_hi_1 = t.raw_geti(hi - 1).unwrap_or(LuaValue::nil());
    let t = table.as_table_mut().unwrap();
    t.raw_seti(hi - 1, pivot);
    t.raw_seti(mid, v_hi_1);

    let mut i = lo;
    let mut j = hi - 1;
    loop {
        // Find element >= pivot from left
        loop {
            i += 1;
            let t = table.as_table().unwrap();
            let v_i = t.raw_geti(i).unwrap_or(LuaValue::nil());
            if !sort_compare(l, v_i, pivot, comp_func, has_comp)? {
                break;
            }
            // If scan went past pivot position, order function is invalid
            if i >= hi - 1 {
                return Err(l.error("invalid order function for sorting".to_string()));
            }
        }
        // Find element <= pivot from right
        loop {
            j -= 1;
            let t = table.as_table().unwrap();
            let v_j = t.raw_geti(j).unwrap_or(LuaValue::nil());
            if !sort_compare(l, pivot, v_j, comp_func, has_comp)? {
                break;
            }
            // If scan went past lo, order function is invalid
            if j <= lo {
                return Err(l.error("invalid order function for sorting".to_string()));
            }
        }
        if i >= j {
            break;
        }
        // Swap
        let t = table.as_table().unwrap();
        let v_i = t.raw_geti(i).unwrap_or(LuaValue::nil());
        let v_j = t.raw_geti(j).unwrap_or(LuaValue::nil());
        let t = table.as_table_mut().unwrap();
        t.raw_seti(i, v_j);
        t.raw_seti(j, v_i);
    }

    // Restore pivot
    let t = table.as_table().unwrap();
    let v_i = t.raw_geti(i).unwrap_or(LuaValue::nil());
    let t = table.as_table_mut().unwrap();
    t.raw_seti(hi - 1, v_i);
    t.raw_seti(i, pivot);

    // Recurse on smaller partition first (tail-call optimization for larger)
    if i - lo < hi - i {
        sort_range(l, table, comp_func, has_comp, lo, i - 1)?;
        sort_range(l, table, comp_func, has_comp, i + 1, hi)?;
    } else {
        sort_range(l, table, comp_func, has_comp, i + 1, hi)?;
        sort_range(l, table, comp_func, has_comp, lo, i - 1)?;
    }

    Ok(())
}
