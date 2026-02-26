use crate::{LuaResult, LuaValue, lua_vm::LuaState};

/// table.sort(list [, comp]) - Sort table in place
///
/// Optimization strategy:
/// 1. Extract elements to Vec using raw array access (O(n) instead of O(n log n) table lookups)
/// 2. For default comparison with homogeneous types: use Rust's built-in pdqsort (sort_unstable_by)
/// 3. For custom comparators or mixed types: fallible introsort (quicksort + insertion sort + heapsort)
/// 4. Write back to table using raw array access
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

    let n = len as usize;

    // === Phase 1: Extract elements to buffer ===
    // Check if table has a metatable — if so, we must use table_geti/table_seti
    // to respect __index/__newindex. If not, raw access is safe and faster.
    let has_meta = table_val
        .as_table_mut()
        .map(|t| t.has_metatable())
        .unwrap_or(false);

    let mut buf: Vec<LuaValue> = Vec::with_capacity(n);
    if has_meta {
        for i in 1..=n {
            let val = l.table_geti(&table_val, i as i64)?;
            buf.push(val);
        }
    } else {
        let table = table_val.as_table_mut().unwrap();
        for i in 1..=n {
            let val = table.raw_geti(i as i64).unwrap_or(LuaValue::nil());
            buf.push(val);
        }
    }

    // Block yields during sort — sort is a non-continuable C call boundary
    l.nny += 1;

    // === Phase 2: Sort the buffer ===
    let result = sort_buffer(l, &mut buf, &comp_func, has_comp);

    // Restore yieldability before returning (even on error)
    l.nny -= 1;

    result?;

    // === Phase 3: Write back to table ===
    if has_meta {
        for (i, val) in buf.into_iter().enumerate() {
            l.table_seti(&table_val, (i + 1) as i64, val)?;
        }
    } else {
        let table = table_val.as_table_mut().unwrap();
        for (i, val) in buf.into_iter().enumerate() {
            table.raw_seti((i + 1) as i64, val);
        }
    }

    // GC write barrier
    if let Some(gc_ptr) = table_val.as_gc_ptr() {
        l.gc_barrier_back(gc_ptr);
    }

    Ok(0)
}

/// Sort the buffer using the best available algorithm.
fn sort_buffer(
    l: &mut LuaState,
    buf: &mut [LuaValue],
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<()> {
    let n = buf.len();
    if n <= 1 {
        return Ok(());
    }

    // === Fast paths for default comparison with homogeneous types ===
    // These use Rust's built-in pdqsort (sort_unstable_by) which is O(n) for
    // sorted/reverse-sorted data and highly optimized with branch-free partitioning.
    if !has_comp {
        let first_tt = buf[0].tt();

        if buf.iter().all(|v| v.tt() == first_tt) {
            // All integers — most common case
            if buf[0].is_integer() {
                buf.sort_unstable_by(|a, b| {
                    let ia = unsafe { a.value.i };
                    let ib = unsafe { b.value.i };
                    ia.cmp(&ib)
                });
                return Ok(());
            }

            // All floats
            if buf[0].is_float() {
                buf.sort_unstable_by(|a, b| {
                    let fa = unsafe { a.value.n };
                    let fb = unsafe { b.value.n };
                    fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
                });
                return Ok(());
            }

            // All strings
            if buf[0].is_string() {
                buf.sort_unstable_by(|a, b| {
                    let sa = a.as_str().unwrap_or("");
                    let sb = b.as_str().unwrap_or("");
                    sa.cmp(sb)
                });
                return Ok(());
            }
        }

        // Mixed numeric types (int + float)
        if buf.iter().all(|v| v.is_integer() || v.is_float()) {
            buf.sort_unstable_by(|a, b| {
                let na = a.as_number().unwrap_or(0.0);
                let nb = b.as_number().unwrap_or(0.0);
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            });
            return Ok(());
        }
    }

    // === General case: fallible introsort ===
    // Used for custom comparators or mixed/incomparable types
    let max_depth = (usize::BITS - n.leading_zeros()) as usize * 2; // ~2 * log2(n)
    introsort(l, buf, 0, n - 1, max_depth, comp_func, has_comp)
}

// ============================================================
// Fallible Introsort Implementation
// Quicksort + Heapsort fallback
// Matches C Lua 5.5's sort semantics (invalid order detection)
// All comparisons return Result to propagate Lua errors
// ============================================================

/// Compare two values: returns Ok(true) if a < b.
#[inline]
fn sort_compare(
    l: &mut LuaState,
    a: LuaValue,
    b: LuaValue,
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<bool> {
    if has_comp {
        l.call_compare(*comp_func, a, b)
    } else {
        l.obj_lt(&a, &b)
    }
}

/// Sort 3 elements at positions lo, mid, hi (median-of-3 for pivot selection).
/// Also handles n<=2 and n<=3 base cases.
#[inline]
fn sort3(
    l: &mut LuaState,
    buf: &mut [LuaValue],
    lo: usize,
    mid: usize,
    hi: usize,
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<()> {
    if sort_compare(l, buf[mid], buf[lo], comp_func, has_comp)? {
        buf.swap(lo, mid);
    }
    if sort_compare(l, buf[hi], buf[mid], comp_func, has_comp)? {
        buf.swap(mid, hi);
        if sort_compare(l, buf[mid], buf[lo], comp_func, has_comp)? {
            buf.swap(lo, mid);
        }
    }
    Ok(())
}

/// Hoare-style partition with median-of-3 pivot.
/// Matches C Lua 5.5's partition() from ltablib.c.
/// Precondition: buf[lo] <= buf[mid] <= buf[hi] (from sort3).
/// Returns the final pivot position.
fn partition(
    l: &mut LuaState,
    buf: &mut [LuaValue],
    lo: usize,
    hi: usize,
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<usize> {
    let mid = lo + (hi - lo) / 2;

    // Sort lo, mid, hi — median goes to mid position
    sort3(l, buf, lo, mid, hi, comp_func, has_comp)?;

    if hi - lo <= 2 {
        return Ok(mid);
    }

    let pivot = buf[mid];
    // Move pivot to hi-1 (out of the way)
    buf.swap(mid, hi - 1);

    // Match C Lua's partition exactly:
    // i starts at lo (will be pre-incremented), j starts at hi-1 (will be pre-decremented)
    let mut i = lo;
    let mut j = hi - 1;

    loop {
        // Left scan: increment then compare, find buf[i] >= pivot
        loop {
            i += 1;
            if !sort_compare(l, buf[i], pivot, comp_func, has_comp)? {
                break;
            }
            // buf[i] < pivot, but if i reached the pivot position, that means
            // pivot < pivot which is an invalid ordering
            if i == hi - 1 {
                return Err(l.error("invalid order function for sorting".to_string()));
            }
        }
        // Right scan: decrement then compare, find buf[j] <= pivot
        loop {
            j -= 1;
            if !sort_compare(l, pivot, buf[j], comp_func, has_comp)? {
                break;
            }
            // pivot < buf[j], but j went past i which contradicts left scan result
            if j < i {
                return Err(l.error("invalid order function for sorting".to_string()));
            }
        }

        if j < i {
            break;
        }
        buf.swap(i, j);
    }

    // Move pivot to its final position
    buf.swap(i, hi - 1);
    Ok(i)
}

/// Heapsort fallback — guarantees O(n log n) worst case.
/// Used when quicksort recursion depth exceeds the limit.
fn heapsort(
    l: &mut LuaState,
    buf: &mut [LuaValue],
    lo: usize,
    hi: usize,
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<()> {
    let n = hi - lo + 1;
    if n <= 1 {
        return Ok(());
    }

    // Build max-heap (sift down from n/2 to 0)
    for i in (0..n / 2).rev() {
        sift_down(l, buf, lo, i, n, comp_func, has_comp)?;
    }

    // Extract elements from heap
    for end in (1..n).rev() {
        buf.swap(lo, lo + end);
        sift_down(l, buf, lo, 0, end, comp_func, has_comp)?;
    }
    Ok(())
}

/// Sift down element at position `pos` in the heap rooted at `lo` with `n` elements.
fn sift_down(
    l: &mut LuaState,
    buf: &mut [LuaValue],
    lo: usize,
    mut pos: usize,
    n: usize,
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<()> {
    loop {
        let left = 2 * pos + 1;
        if left >= n {
            break;
        }
        let right = left + 1;
        let mut largest = pos;

        if sort_compare(l, buf[lo + largest], buf[lo + left], comp_func, has_comp)? {
            largest = left;
        }
        if right < n && sort_compare(l, buf[lo + largest], buf[lo + right], comp_func, has_comp)? {
            largest = right;
        }
        if largest == pos {
            break;
        }
        buf.swap(lo + pos, lo + largest);
        pos = largest;
    }
    Ok(())
}

/// Introsort: quicksort with depth limit.
/// Matches C Lua 5.5's auxsort behavior:
/// - n <= 1: nothing
/// - n == 2: compare and swap
/// - n == 3: sort3
/// - n >= 4: sort3 + partition + recurse (invalid order detected here)
/// Falls back to heapsort when recursion is too deep (O(n log n) guaranteed).
fn introsort(
    l: &mut LuaState,
    buf: &mut [LuaValue],
    mut lo: usize,
    mut hi: usize,
    mut depth_limit: usize,
    comp_func: &LuaValue,
    has_comp: bool,
) -> LuaResult<()> {
    while lo < hi {
        let n = hi - lo + 1;

        // n == 2: compare and swap
        if n == 2 {
            if sort_compare(l, buf[hi], buf[lo], comp_func, has_comp)? {
                buf.swap(lo, hi);
            }
            return Ok(());
        }

        // n == 3: sort3
        if n == 3 {
            let mid = lo + 1;
            sort3(l, buf, lo, mid, hi, comp_func, has_comp)?;
            return Ok(());
        }

        // n >= 4: use partition (which detects invalid order functions)
        // Depth limit exceeded: fall back to heapsort (O(n log n) guaranteed)
        if depth_limit == 0 {
            return heapsort(l, buf, lo, hi, comp_func, has_comp);
        }
        depth_limit -= 1;

        // Partition
        let p = partition(l, buf, lo, hi, comp_func, has_comp)?;

        // Recurse on smaller partition, tail-call on larger (stack depth = O(log n))
        if p.saturating_sub(lo) < hi.saturating_sub(p) {
            if p > lo {
                introsort(l, buf, lo, p - 1, depth_limit, comp_func, has_comp)?;
            }
            lo = p + 1;
        } else {
            if p < hi {
                introsort(l, buf, p + 1, hi, depth_limit, comp_func, has_comp)?;
            }
            if p == 0 {
                break;
            }
            hi = p - 1;
        }
    }
    Ok(())
}
