// Native Lua 5.5-style table implementation
// Port of ltable.c with minimal abstractions for maximum performance

use crate::lua_value::{
    LuaValue,
    lua_value::{LUA_VEMPTY, LUA_VNIL, Value},
};

use std::alloc::{self, Layout};
use std::ptr;

/// Node for hash table - mimics Lua 5.5's Node structure
/// Key-Value pair + next pointer for collision chaining
#[derive(Clone, Copy)]
#[repr(C)]
struct Node {
    /// Value stored in this node
    value: LuaValue,
    /// Key stored in this node  
    key: LuaValue,
    /// Next node in collision chain (offset, 0 = end)
    next: i32,
}

/// Native Lua table implementation - mimics Lua 5.5's Table struct
///
/// Array layout (Lua 5.5 optimization):
/// ```md,ignore
///      Values                          Tags
/// ----------------------------------------
/// ... | Val1 | Val0 | lenhint | 0 | 1 | ...
/// ----------------------------------------
///                    ^ array pointer
/// ```
/// - Values are accessed with negative offsets: array[-1-k]
/// - Tags are accessed with positive offsets: array[sizeof(u32) + k]
/// - This saves 43% memory vs storing full TValue structs
pub struct NativeTable {
    /// Array pointer - points BETWEEN values and tags (PUBLIC for VM hot path)
    pub(crate) array: *mut u8,
    /// Array size in elements (PUBLIC for VM hot path)
    pub(crate) asize: u32,

    /// Hash part (Node array)
    node: *mut Node,
    /// log2 of hash size (size = 1 << lsizenode)
    lsizenode: u8,
    /// Last free position in hash table (optimization like Lua 5.5)
    /// Points to next candidate for free slot search
    lastfree: *mut Node,
}

impl NativeTable {
    /// Create new table with given capacity
    pub fn new(array_cap: u32, hash_cap: u32) -> Self {
        let mut table = Self {
            array: ptr::null_mut(),
            asize: 0,
            node: ptr::null_mut(),
            lsizenode: 0,
            lastfree: ptr::null_mut(),
        };

        // Allocate array part
        if array_cap > 0 {
            table.resize_array(array_cap);
        }

        // Allocate hash part
        if hash_cap > 0 {
            let lsize = Self::compute_lsizenode(hash_cap);
            table.resize_hash(lsize);
        }

        table
    }

    /// Compute log2(size) for hash part
    #[inline]
    fn compute_lsizenode(size: u32) -> u8 {
        if size == 0 {
            return 0;
        }
        let mut lsize = 0u8;
        let mut s = size - 1;
        while s > 0 {
            s >>= 1;
            lsize += 1;
        }
        lsize
    }

    /// Get hash size (number of nodes)
    #[inline(always)]
    fn sizenode(&self) -> usize {
        if self.node.is_null() {
            0
        } else {
            1usize << self.lsizenode
        }
    }

    #[inline(always)]
    fn is_dummy(&self) -> bool {
        self.node.is_null()
    }

    /// Check if hash part is non-empty (cheaper than sizenode() > 0)
    #[inline(always)]
    pub fn has_hash(&self) -> bool {
        !self.node.is_null()
    }

    /// Get main position WITHOUT checking for empty hash (caller must guarantee has_hash())
    /// Uses general hash_value which handles all key types.
    #[inline(always)]
    fn mainposition_fast(&self, key: &LuaValue) -> *mut Node {
        let hash = key.hash_value();
        let mask = (1usize << self.lsizenode) - 1;
        unsafe { self.node.add((hash as usize) & mask) }
    }

    /// Get main position for string keys only — skips type check in hash computation.
    /// SAFETY: caller must guarantee key is a string AND hash is non-empty.
    #[inline(always)]
    fn mainposition_string(&self, key: &LuaValue) -> *mut Node {
        let hash = unsafe { key.hash_string_unchecked() };
        let mask = (1usize << self.lsizenode) - 1;
        unsafe { self.node.add((hash as usize) & mask) }
    }

    /// Fast short string lookup — assumes hash is non-empty and key is short string.
    /// Caller must guarantee both conditions.
    #[inline(always)]
    pub fn get_shortstr_unchecked(&self, key: &LuaValue) -> Option<LuaValue> {
        let mut node = self.mainposition_string(key);
        let key_ptr = unsafe { key.value.i };

        unsafe {
            // Unroll first iteration (most common case: found in main position)
            if (*node).key.is_string() && (*node).key.value.i == key_ptr {
                let val = (*node).value;
                return if val.is_nil() { None } else { Some(val) };
            }

            let mut next = (*node).next;
            while next != 0 {
                node = node.offset(next as isize);
                if (*node).key.is_string() && (*node).key.value.i == key_ptr {
                    let val = (*node).value;
                    return if val.is_nil() { None } else { Some(val) };
                }
                next = (*node).next;
            }
            None
        }
    }

    /// Fast short string SET — assumes hash is non-empty and key is short string.
    /// Only updates existing keys (returns false if key not found).
    /// Caller must guarantee hash is non-empty.
    #[inline(always)]
    pub fn set_shortstr_unchecked(&mut self, key: &LuaValue, value: LuaValue) -> bool {
        let mp = self.mainposition_string(key);
        let key_ptr = unsafe { key.value.i };

        unsafe {
            // Check main position first
            if (*mp).key.is_string() && (*mp).key.value.i == key_ptr {
                (*mp).value = value;
                return true;
            }

            // Walk collision chain
            let mut node = mp;
            let mut next = (*node).next;
            while next != 0 {
                node = node.offset(next as isize);
                if (*node).key.is_string() && (*node).key.value.i == key_ptr {
                    (*node).value = value;
                    return true;
                }
                next = (*node).next;
            }
        }
        false
    }

    /// Get pointer to tag for array index k (0-based C index)
    #[inline(always)]
    unsafe fn get_arr_tag(&self, k: usize) -> *mut u8 {
        // array + sizeof(u32) + k
        unsafe { self.array.add(std::mem::size_of::<u32>() + k) }
    }

    /// Get pointer to value for array index k (0-based C index)
    #[inline(always)]
    unsafe fn get_arr_val(&self, k: usize) -> *mut Value {
        // array - 1 - k (in Value units)
        let value_ptr = self.array as *mut Value;
        unsafe { value_ptr.sub(1 + k) }
    }

    /// Get lenhint pointer
    #[inline(always)]
    unsafe fn lenhint_ptr(&self) -> *mut u32 {
        self.array as *mut u32
    }

    /// Read value from array at Lua index (1-based)
    #[inline(always)]
    unsafe fn read_array(&self, lua_index: i64) -> Option<LuaValue> {
        if lua_index < 1 || lua_index > self.asize as i64 {
            return None;
        }
        let k = (lua_index - 1) as usize; // Convert to 0-based C index

        unsafe {
            let tt = *self.get_arr_tag(k);

            // Check if empty
            if tt == LUA_VNIL || tt == LUA_VEMPTY {
                return None;
            }

            let val_ptr = self.get_arr_val(k);
            let value = *val_ptr;

            Some(LuaValue { value, tt })
        }
    }

    /// Write value to array at Lua index (1-based)
    #[inline(always)]
    pub unsafe fn write_array(&mut self, lua_index: i64, luaval: LuaValue) {
        if lua_index < 1 || lua_index > self.asize as i64 {
            return;
        }
        let k = (lua_index - 1) as usize; // Convert to 0-based C index

        unsafe {
            *self.get_arr_tag(k) = luaval.tt;
            *self.get_arr_val(k) = luaval.value;

            // Update lenhint — tracks the length of the initial contiguous
            // non-nil sequence from index 1 (i.e., the largest i such that
            // a[1]..a[i] are all non-nil).
            let lenhint = *self.lenhint_ptr();

            if !luaval.is_nil() {
                // Adding a non-nil value
                if lua_index == lenhint as i64 + 1 {
                    // Extending the sequence — scan forward past any
                    // existing non-nil elements to find the true boundary
                    let mut new_lenhint = lua_index as u32;
                    let asize = self.asize;
                    while new_lenhint < asize {
                        let tag = *self.get_arr_tag(new_lenhint as usize);
                        if tag == LUA_VNIL || tag == LUA_VEMPTY {
                            break;
                        }
                        new_lenhint += 1;
                    }
                    *self.lenhint_ptr() = new_lenhint;
                }
                // If lua_index > lenhint+1: there's a hole, lenhint unchanged
                // If lua_index <= lenhint: already in the sequence, lenhint unchanged
            } else {
                // Setting to nil — if within the current sequence, truncate
                if lua_index <= lenhint as i64 {
                    *self.lenhint_ptr() = (lua_index as u32) - 1;
                }
            }
        }
    }

    /// Resize array part
    pub(crate) fn resize_array(&mut self, new_size: u32) {
        if new_size == 0 {
            if !self.array.is_null() && self.asize > 0 {
                // Free old array
                // Layout: [Values...][lenhint][Tags...]
                let values_size = self.asize as usize * std::mem::size_of::<Value>();
                let lenhint_size = std::mem::size_of::<u32>();
                let tags_size = self.asize as usize;
                let total_size = values_size + lenhint_size + tags_size;

                // array pointer points to lenhint, need to go back to start
                let start_ptr = unsafe { self.array.sub(values_size) };
                let layout =
                    Layout::from_size_align(total_size, std::mem::align_of::<Value>()).unwrap();
                unsafe { alloc::dealloc(start_ptr, layout) };
            }
            self.array = ptr::null_mut();
            self.asize = 0;
            return;
        }

        let old_size = self.asize;

        // Calculate sizes
        let values_size = new_size as usize * std::mem::size_of::<Value>();
        let lenhint_size = std::mem::size_of::<u32>();
        let tags_size = new_size as usize; // Each tag is 1 byte
        let total_size = values_size + lenhint_size + tags_size;

        // Allocate new memory
        let layout = Layout::from_size_align(total_size, std::mem::align_of::<Value>()).unwrap();
        let start_ptr = unsafe { alloc::alloc(layout) };
        if start_ptr.is_null() {
            panic!("Failed to allocate array");
        }

        // Set array pointer to point at lenhint position
        let new_array = unsafe { start_ptr.add(values_size) };

        // Initialize lenhint to 0
        unsafe {
            *(new_array as *mut u32) = 0;
        }

        // Initialize all tags to LUA_VNIL (0) and all values to 0 — single memset
        // Since tags are at new_array+4 and values at start_ptr, and LUA_VNIL == 0,
        // we can zero the entire allocation in one shot.
        unsafe {
            ptr::write_bytes(start_ptr, 0, total_size);
        }

        // Copy old data if exists
        if !self.array.is_null() && old_size > 0 {
            let copy_size = old_size.min(new_size) as usize;

            unsafe {
                // Copy values - values are stored backward from array pointer
                // Source: copy the FIRST copy_size values (indices 1..copy_size)
                // V[0]..V[copy_size-1], at array - sizeof(Value) .. array - copy_size*sizeof(Value)
                let old_values_start =
                    self.array.sub(copy_size * std::mem::size_of::<Value>()) as *const Value;
                let new_values_end = new_array.sub(std::mem::size_of::<Value>()) as *mut Value;
                let new_values_start_for_copy = new_values_end.sub(copy_size - 1);
                ptr::copy_nonoverlapping(old_values_start, new_values_start_for_copy, copy_size);

                // Copy tags
                let old_tags = self.array.add(std::mem::size_of::<u32>());
                let new_tags = new_array.add(std::mem::size_of::<u32>());
                ptr::copy_nonoverlapping(old_tags, new_tags, copy_size);

                // Copy lenhint
                let old_lenhint = *(self.array as *const u32);
                *(new_array as *mut u32) = old_lenhint.min(new_size);
            }

            // Free old array
            let old_values_size = old_size as usize * std::mem::size_of::<Value>();
            let old_start = unsafe { self.array.sub(old_values_size) };
            let old_total = old_values_size + lenhint_size + old_size as usize;
            let old_layout =
                Layout::from_size_align(old_total, std::mem::align_of::<Value>()).unwrap();
            unsafe { alloc::dealloc(old_start, old_layout) };
        }

        self.array = new_array;
        self.asize = new_size;
    }

    /// Resize hash part
    fn resize_hash(&mut self, new_lsize: u8) {
        let old_size = self.sizenode();
        let new_size = if new_lsize == 0 {
            0
        } else {
            1usize << new_lsize
        };

        let old_node = self.node;
        let was_dummy = self.is_dummy();

        if new_size == 0 {
            // Switch to dummy
            if !was_dummy && old_size > 0 {
                let layout = Layout::array::<Node>(old_size).unwrap();
                unsafe { alloc::dealloc(old_node as *mut u8, layout) };
            }
            self.node = ptr::null_mut();
            self.lsizenode = 0;
            return;
        }

        // Allocate new hash array
        let layout = Layout::array::<Node>(new_size).unwrap();
        let new_node = unsafe { alloc::alloc(layout) as *mut Node };
        if new_node.is_null() {
            panic!("Failed to allocate hash nodes");
        }

        // Initialize all nodes
        unsafe {
            for i in 0..new_size {
                let node = new_node.add(i);
                ptr::write(
                    node,
                    Node {
                        value: LuaValue::nil(),
                        key: LuaValue::nil(),
                        next: 0,
                    },
                );
            }
        }

        self.node = new_node;
        self.lsizenode = new_lsize;
        // Initialize lastfree to end of node array (Lua 5.5 optimization)
        self.lastfree = unsafe { new_node.add(new_size) };

        // Rehash old entries - CRITICAL: Use raw_set to respect array/hash invariant
        // lua5.5's reinserthash calls newcheckedkey which checks keyinarray
        // CRITICAL: Only rehash LIVE entries (non-empty value).
        // Dead keys (key non-nil, value nil) must be skipped because:
        // 1. The GC does not mark strings referenced only by dead keys
        // 2. Those strings may have been collected (freed)
        // 3. Hashing a freed string → use-after-free → crash (0xC0000005)
        // This matches C Lua 5.5's reinserthash: `if (!isempty(gval(old)))`
        if !was_dummy && old_size > 0 {
            for i in 0..old_size {
                unsafe {
                    let old_n = old_node.add(i);
                    // Only rehash entries with live values (skip dead keys)
                    if !(*old_n).key.is_nil() && !(*old_n).value.is_nil() {
                        let key = (*old_n).key;
                        let value = (*old_n).value;
                        // Must use raw_set here, not set_node!
                        // raw_set will put integer keys in [1..asize] into array part only
                        self.raw_set(&key, value);
                    }
                }
            }

            let old_layout = Layout::array::<Node>(old_size).unwrap();
            unsafe { alloc::dealloc(old_node as *mut u8, old_layout) };
        }
    }

    /// Port of Lua 5.5's rehash from ltable.c
    /// Computes optimal sizes for array and hash parts, then resizes.
    /// Called when hash part is full and a new key needs to be inserted.
    ///
    /// This ensures integer keys are properly distributed between array and
    /// hash parts, even when the table is sparse (e.g., after GC clearing
    /// entries from a weak table).
    fn rehash(&mut self, extra_key: &LuaValue) {
        const MAXABITS: usize = 30; // max bits for array index (Lua 5.5 uses 26-31)

        // nums[i] = number of integer keys k where 2^(i-1) < k <= 2^i
        // nums[0] = number of keys with k == 1 (i.e., 2^0)
        let mut nums = [0u32; MAXABITS + 1];

        // Count integer keys in array part
        let mut na = self.numusearray(&mut nums);
        let mut totaluse = na as usize;

        // Count integer keys in hash part
        let (hash_use, has_deleted) = self.numusehash(&mut nums, &mut na);
        totaluse += hash_use;

        // Count the extra key (the key being inserted)
        if extra_key.ttisinteger() {
            let k = extra_key.ivalue();
            if k >= 1 && (k as u64) <= (1u64 << MAXABITS) {
                na += 1;
                // Find which bin: ceil(log2(k))
                let bin = if k == 1 {
                    0
                } else {
                    64 - ((k - 1) as u64).leading_zeros() as usize
                };
                if bin <= MAXABITS {
                    nums[bin] += 1;
                }
            }
        }
        totaluse += 1; // count the extra key

        // Compute optimal array size
        let (optimal_asize, na_in_array) = Self::computesizes(&nums, na);

        // Number of entries for hash part
        let mut hash_entries = totaluse - na_in_array as usize;

        // Lua 5.5 optimization: if dead keys were found (insertion-deletion
        // pattern), give hash part 25% extra capacity to avoid repeated
        // rehashes. Matches C Lua 5.5's `nsize += nsize >> 2`.
        if has_deleted {
            hash_entries += hash_entries >> 2;
        }

        // Resize both parts
        self.resize(optimal_asize, hash_entries as u32);
    }

    /// Port of Lua 5.5's numusearray
    /// Count integer keys in array part, populating nums[] bins.
    fn numusearray(&self, nums: &mut [u32]) -> u32 {
        let mut ause = 0u32; // total non-nil integer keys in array
        let asize = self.asize;

        if asize == 0 {
            return 0;
        }

        // Iterate through array and count non-nil entries per power-of-2 bin
        // Port of Lua 5.5's numusearray: must handle asize not being a power of 2.
        let mut twotoi = 1u32; // 2^i
        let mut bin = 0usize;
        let mut i = 1u32; // lua index (1-based)

        while bin < nums.len() {
            let mut limit = twotoi;
            if limit > asize {
                limit = asize;
                if i > limit {
                    break; // no more elements to count
                }
            }
            // Count entries in range (twotoi/2, twotoi] clamped to asize
            while i <= limit {
                unsafe {
                    let k = (i - 1) as usize;
                    let tag = *self.get_arr_tag(k);
                    if tag != LUA_VNIL && tag != LUA_VEMPTY {
                        ause += 1;
                        nums[bin] += 1;
                    }
                }
                i += 1;
            }
            bin += 1;
            twotoi = twotoi.wrapping_mul(2);
        }

        ause
    }

    /// Port of Lua 5.5's numusehash
    /// Count total entries in hash part, and for integer keys, add to nums[].
    /// Returns (totaluse, has_deleted) — has_deleted is true if any dead keys exist
    /// (key non-nil but value nil), matching C Lua 5.5's `ct.deleted` flag.
    fn numusehash(&self, nums: &mut [u32], na: &mut u32) -> (usize, bool) {
        let size = self.sizenode();
        let mut totaluse = 0usize;
        let mut has_deleted = false;

        for i in 0..size {
            unsafe {
                let node = self.node.add(i);
                let key = &(*node).key;
                if key.is_nil() {
                    continue;
                }
                if (*node).value.is_nil() {
                    // Dead key: key is non-nil but value is nil
                    has_deleted = true;
                    continue;
                }
                totaluse += 1;
                // Check if key is a positive integer
                if key.ttisinteger() {
                    let k = key.ivalue();
                    if k >= 1 && (k as u64) <= (1u64 << 30) {
                        *na += 1;
                        let bin = if k == 1 {
                            0
                        } else {
                            64 - ((k - 1) as u64).leading_zeros() as usize
                        };
                        if bin < nums.len() {
                            nums[bin] += 1;
                        }
                    }
                }
            }
        }

        (totaluse, has_deleted)
    }

    /// Port of Lua 5.5's computesizes
    /// Compute optimal array size: largest power of 2 such that
    /// more than half the slots would be filled.
    /// Returns (optimal_array_size, count_of_integer_keys_going_to_array).
    fn computesizes(nums: &[u32], na: u32) -> (u32, u32) {
        let mut a = 0u32; // count of elements <= 2^i
        let mut na_final = 0u32; // elements going to array
        let mut optimal = 0u32; // optimal array size

        let mut twotoi = 1u32; // 2^i candidate size

        for i in 0..nums.len() {
            if twotoi == 0 {
                break;
            } // overflow
            if na <= twotoi / 2 {
                break;
            } // remaining keys can't fill half

            a += nums[i];
            if a > twotoi / 2 {
                // More than half elements present → good size
                optimal = twotoi;
                na_final = a;
            }
            twotoi = twotoi.wrapping_mul(2);
        }

        (optimal, na_final)
    }

    /// Resize both array and hash parts. Port of Lua 5.5's luaH_resize.
    /// Moves integer keys to array and non-integer keys to hash.
    fn resize(&mut self, new_asize: u32, new_hash_count: u32) {
        let old_asize = self.asize;

        // Shrink array if needed: move excess array entries to hash
        // (not common but Lua 5.5 supports it)
        if new_asize < old_asize {
            // First, resize hash to accommodate new entries
            let new_lsize = if new_hash_count > 0 {
                Self::compute_lsizenode(new_hash_count)
            } else {
                0
            };
            self.resize_hash(new_lsize);

            // Move array entries [new_asize+1..old_asize] to hash
            for i in (new_asize + 1)..=old_asize {
                unsafe {
                    if let Some(val) = self.read_array(i as i64) {
                        let key = LuaValue::integer(i as i64);
                        self.set_node(key, val);
                    }
                }
            }
            self.resize_array(new_asize);
        } else {
            // Grow or keep array the same
            if new_asize > old_asize {
                self.resize_array(new_asize);
            }

            // Resize hash
            let new_lsize = if new_hash_count > 0 {
                Self::compute_lsizenode(new_hash_count)
            } else {
                0
            };

            // Save old hash data
            let save_node = self.node;
            let save_size = self.sizenode();
            let save_dummy = self.is_dummy();

            // Allocate new hash
            if new_lsize > 0 {
                let new_size = 1usize << new_lsize;
                let layout = Layout::array::<Node>(new_size).unwrap();
                let new_node = unsafe { alloc::alloc(layout) as *mut Node };
                if new_node.is_null() {
                    panic!("Failed to allocate hash nodes");
                }
                unsafe {
                    for i in 0..new_size {
                        let node = new_node.add(i);
                        ptr::write(
                            node,
                            Node {
                                value: LuaValue::nil(),
                                key: LuaValue::nil(),
                                next: 0,
                            },
                        );
                    }
                }
                self.node = new_node;
                self.lsizenode = new_lsize;
                self.lastfree = unsafe { new_node.add(new_size) };
            } else {
                self.node = ptr::null_mut();
                self.lsizenode = 0;
                self.lastfree = ptr::null_mut();
            }

            // Re-insert old hash entries
            if !save_dummy && save_size > 0 {
                for i in 0..save_size {
                    unsafe {
                        let old_n = save_node.add(i);
                        if !(*old_n).key.is_nil() && !(*old_n).value.is_nil() {
                            let key = (*old_n).key;
                            let value = (*old_n).value;
                            // Use raw_set: integer keys in [1..new_asize] go to array
                            self.raw_set(&key, value);
                        }
                    }
                }
                let old_layout = Layout::array::<Node>(save_size).unwrap();
                unsafe { alloc::dealloc(save_node as *mut u8, old_layout) };
            }
        }
    }

    /// Get main position for a key (hash index)
    #[inline(always)]
    fn mainposition(&self, key: &LuaValue) -> *mut Node {
        let size = self.sizenode();
        if size == 0 {
            return self.node;
        }

        let hash = key.hash_value();
        let index = (hash as usize) & (size - 1); // size is power of 2

        unsafe { self.node.add(index) }
    }

    /// Fast GETI path - mirrors Lua 5.5's luaH_fastgeti macro
    /// CRITICAL: This must be #[inline(always)] for zero-cost abstraction
    /// Called directly from VM execute loop for maximum performance
    #[inline(always)]
    pub fn fast_geti(&self, key: i64) -> Option<LuaValue> {
        // Fast path: array bounds check
        if key >= 1 && key <= self.asize as i64 {
            let k = (key - 1) as usize;
            unsafe {
                // Direct array access (zero function calls)
                // Layout: Tags at array + sizeof(u32) + k
                //         Values at array - sizeof(Value) * (1 + k)
                let tag_ptr = (self.array as *const u8).add(4 + k);
                let tt = *tag_ptr;

                if tt != LUA_VNIL && tt != LUA_VEMPTY {
                    let value_ptr = (self.array as *mut Value).sub(1 + k);
                    let value = *value_ptr;
                    return Some(LuaValue { value, tt });
                }
            }
            return None;
        }

        // Slow path: hash part lookup
        if self.sizenode() > 0 {
            let key_val = LuaValue::integer(key);
            return self.get_from_hash(&key_val);
        }

        None
    }

    /// Get value from array part
    /// OPTIMIZED: Inline array access for maximum performance
    #[inline(always)]
    pub fn get_int(&self, key: i64) -> Option<LuaValue> {
        // Delegate to fast_geti for consistency
        self.fast_geti(key)
    }

    /// Fast SETI path - mirrors Lua 5.5's luaH_fastseti macro
    /// CRITICAL: This must be #[inline(always)] for zero-cost abstraction
    #[inline(always)]
    pub fn fast_seti(&mut self, key: i64, value: LuaValue) -> bool {
        // Fast path: array bounds check
        if key >= 1 && key <= self.asize as i64 {
            unsafe {
                self.write_array(key, value);
            }
            return true;
        }
        false
    }

    /// Fast SETI variant that only succeeds when an existing non-nil value exists.
    /// Safe to use when the table has a metatable, because __newindex is only
    /// consulted for keys whose rawget() returns nil.
    #[inline(always)]
    pub fn fast_seti_existing(&mut self, key: i64, value: LuaValue) -> bool {
        if key >= 1 && key <= self.asize as i64 {
            let existing = unsafe { self.read_array(key) };
            if existing.is_some_and(|v| !v.is_nil()) {
                unsafe {
                    self.write_array(key, value);
                }
                return true;
            }
        }
        false
    }

    /// Set value in array part
    #[inline(always)]
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        // Try fast path first
        if self.fast_seti(key, value) {
            return;
        }

        self.set_int_slow(key, value);
    }

    /// Slow path of set_int — called when fast_seti already failed.
    /// Handles resize/push and hash fallback.
    #[inline(always)]
    pub fn set_int_slow(&mut self, key: i64, value: LuaValue) {
        // Key outside array range: push optimization for sequential insertion
        if key >= 1 && !value.is_nil() {
            // Fast check: key == asize + 1 means appending right after array end.
            // This covers the vast majority of sequential push cases (t[i] = i in a loop).
            let asize = self.asize as i64;
            if key == asize + 1 || (key > asize && key == self.len() as i64 + 1) {
                // This is a push operation, expand array
                let new_size = ((key as u32).next_power_of_two()).max(4);
                self.resize_array(new_size);
                unsafe {
                    self.write_array(key, value);
                }
                // After expanding, migrate any integer keys from hash that now
                // fit in the new array range. Without this, keys already in hash
                // (e.g., r[3]=true; r[5]=true; r[1]=true) would become invisible
                // to raw_get since it checks array first for in-range keys.
                // Skip when hash is empty (common case for sequential insertion).
                if !self.node.is_null() {
                    self.migrate_hash_int_keys_to_array();
                }
                return;
            }
        }

        // Put in hash part (rehash will rebalance later if needed)
        let key_val = LuaValue::integer(key);
        self.set_node(key_val, value);
    }

    /// Fast GETFIELD path - for short string keys (most common in field access)
    /// CRITICAL: This must be #[inline(always)] for zero-cost abstraction
    #[inline(always)]
    pub fn fast_getfield(&self, key: &LuaValue) -> Option<LuaValue> {
        // GETFIELD only uses short string keys (interned)
        if key.is_short_string() {
            return self.get_shortstr_fast(key);
        }

        None
    }

    /// Fast SETFIELD path - for short string keys
    /// Returns true if successfully set in hash part (no metatable, key exists or room available)
    /// CRITICAL: This must be #[inline(always)] for zero-cost abstraction
    #[inline(always)]
    pub fn fast_setfield(&mut self, key: &LuaValue, value: LuaValue) -> bool {
        // Only handles short strings (SETFIELD always uses short string keys)
        if !key.is_short_string() {
            return false;
        }

        if self.node.is_null() {
            // Need to allocate - can't do in fast path
            return false;
        }

        // Try to find existing key or free slot in main position
        let mp = self.mainposition_string(key);
        let key_ptr = unsafe { key.value.i };

        unsafe {
            // Check main position first
            if (*mp).key.is_string() && (*mp).key.value.i == key_ptr {
                // Key matches at main position
                if (*mp).value.is_nil() {
                    // Dead key (value=nil): fall through to slow path.
                    // The slow path calls invalidate_tm_cache() which is
                    // needed when re-inserting keys like "__eq" into metatables.
                    return false;
                }
                // Live key - update value in place
                (*mp).value = value;
                return true;
            }

            // If main position is free (nil key), fall through to slow path
            // for TM cache invalidation on new key insertion.
            if (*mp).key.is_nil() {
                return false;
            }

            // Check collision chain
            let mut node = mp;
            let mut next = (*node).next;
            while next != 0 {
                node = node.offset(next as isize);
                if (*node).key.is_string() && (*node).key.value.i == key_ptr {
                    if (*node).value.is_nil() {
                        // Dead key in chain - slow path for TM cache invalidation
                        return false;
                    }
                    // Live key in chain - update value
                    (*node).value = value;
                    return true;
                }
                next = (*node).next;
            }
        }

        // Key not found and complex insertion needed - use slow path
        false
    }

    /// Get value from hash part - CRITICAL HOT PATH
    #[inline(always)]
    fn get_from_hash(&self, key: &LuaValue) -> Option<LuaValue> {
        if self.node.is_null() {
            return None;
        }

        // Fast path for short strings only - direct pointer comparison
        // Long strings (>40 chars) are NOT interned, so must use general case
        if key.is_short_string() {
            return self.get_shortstr_fast(key);
        }

        // General case (includes long strings)
        let mut node = self.mainposition_fast(key);

        loop {
            unsafe {
                // CRITICAL: Skip dead keys (value is nil) to avoid use-after-free.
                // Dead keys may reference freed GC objects (e.g., long strings).
                // Comparing them would dereference dangling pointers → crash.
                // This matches Lua 5.5's dead key handling (LUA_TDEADKEY type).
                if !(*node).value.is_nil() && (*node).key == *key {
                    return Some((*node).value);
                }

                let next = (*node).next;
                if next == 0 {
                    return None;
                }
                node = node.offset(next as isize);
            }
        }
    }

    /// Fast path for short string lookup - mimics luaH_Hgetshortstr.
    /// Public so metatable TM lookups can bypass raw_get's float normalization.
    /// OPTIMIZED: Reduced branches in hot loop, pointer-equality for interned strings.
    /// Safe to call on empty hash tables (returns None).
    #[inline(always)]
    pub fn get_shortstr_fast(&self, key: &LuaValue) -> Option<LuaValue> {
        if self.node.is_null() {
            return None;
        }
        let mut node = self.mainposition_string(key);
        let key_ptr = unsafe { key.value.i };

        unsafe {
            // Unroll first iteration (most common case: found in main position)
            if (*node).key.is_string() && (*node).key.value.i == key_ptr {
                let val = (*node).value;
                return if val.is_nil() { None } else { Some(val) };
            }

            let mut next = (*node).next;
            while next != 0 {
                node = node.offset(next as isize);
                // Short strings: pointer comparison only (interned)
                if (*node).key.is_string() && (*node).key.value.i == key_ptr {
                    let val = (*node).value;
                    return if val.is_nil() { None } else { Some(val) };
                }
                next = (*node).next;
            }
            None
        }
    }

    /// Generic get
    #[inline(always)]
    pub fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        // Normalize float keys to integer if they have no fractional part
        // This ensures t[3.0] and t[3] refer to the same slot
        // Only check actual float type (ttisfloat), not integers
        let mut key = *key;
        if key.ttisfloat() {
            let f = key.fltvalue();
            if f.fract() == 0.0 && f.is_finite() {
                let i = f as i64;
                if i as f64 == f {
                    key = LuaValue::integer(i);
                }
            }
        };

        // Try array part for integers
        if key.ttisinteger() {
            let i = key.ivalue();
            unsafe {
                if let Some(val) = self.read_array(i) {
                    return Some(val);
                }
            }
        }

        // Hash part
        self.get_from_hash(&key)
    }

    /// After expanding the array part, move any integer keys from the hash
    /// that now fall within [1..asize] into the array. This maintains the
    /// invariant that integer keys in array range are stored ONLY in the
    /// array part (otherwise `fast_geti`/`get_int` would miss them).
    fn migrate_hash_int_keys_to_array(&mut self) {
        let size = self.sizenode();
        if size == 0 {
            return;
        }
        let asize = self.asize as i64;
        // Collect keys to migrate (can't modify hash while iterating)
        let mut to_migrate: Vec<(i64, LuaValue)> = Vec::new();
        for i in 0..size {
            unsafe {
                let node = self.node.add(i);
                if !(*node).key.is_nil() && !(*node).value.is_nil() && (*node).key.ttisinteger() {
                    let k = (*node).key.ivalue();
                    if k >= 1 && k <= asize {
                        to_migrate.push((k, (*node).value));
                    }
                }
            }
        }
        // Move each key: write to array, remove from hash
        for (k, v) in to_migrate {
            unsafe {
                self.write_array(k, v);
            }
            let key_val = LuaValue::integer(k);
            self.set_node(key_val, LuaValue::nil()); // mark dead in hash
        }
    }

    /// Set value in hash part
    /// Find a free position in hash table (Lua 5.5 optimization with lastfree)
    fn getfreepos(&mut self) -> Option<*mut Node> {
        if self.sizenode() == 0 {
            return None;
        }

        unsafe {
            // Search backwards from lastfree (Lua 5.5 pattern)
            while self.lastfree > self.node {
                self.lastfree = self.lastfree.offset(-1);
                if (*self.lastfree).key.is_nil() {
                    return Some(self.lastfree);
                }
            }
        }

        None // Table is full
    }

    /// Compute the node offset from `from` to `to` as an i32.
    /// Equivalent to C Lua's `cast_int(to - from)` for Node pointers.
    #[inline(always)]
    fn node_offset(from: *mut Node, to: *mut Node) -> i32 {
        ((to as isize - from as isize) / std::mem::size_of::<Node>() as isize) as i32
    }

    fn set_node(&mut self, key: LuaValue, value: LuaValue) {
        // If setting to nil, find existing node and only clear value (keep key for next() iteration)
        if value.is_nil() {
            if self.sizenode() == 0 {
                return;
            }
            unsafe {
                let mp = self.mainposition(&key);
                let mut node = mp;
                loop {
                    // Skip dead keys (value nil) - avoid UAF on freed strings
                    if !(*node).value.is_nil() && (*node).key == key {
                        (*node).value = LuaValue::nil();
                        return;
                    }
                    let next = (*node).next;
                    if next == 0 {
                        return; // Key not found, nothing to do
                    }
                    node = node.offset(next as isize);
                }
            }
        }

        if self.sizenode() == 0 {
            // Need to allocate hash part
            self.resize_hash(2); // Start with 4 nodes
        }

        let mp = self.mainposition(&key);

        unsafe {
            // If main position is free (nil key) or dead (nil value), use it.
            // Matches C Lua 5.5: `if (!isempty(gval(mp)))`.
            // CRITICAL: Dead keys must be treated as free to avoid hashing their
            // (potentially GC-collected) string keys in mainposition() below.
            // When reusing a dead key's slot, preserve the `next` link so that
            // any chain passing through this slot remains intact.
            if (*mp).key.is_nil() {
                (*mp).key = key;
                (*mp).value = value;
                (*mp).next = 0;
                return;
            }
            if (*mp).value.is_nil() {
                // Dead key: main position slot is available for reuse.
                // BUT we must first check if this key already exists alive
                // somewhere in the chain — otherwise we create a duplicate.
                // Walk the chain starting from mp's next link.
                let mut scan = mp;
                loop {
                    let next_off = (*scan).next;
                    if next_off == 0 {
                        break;
                    }
                    scan = scan.offset(next_off as isize);
                    if !(*scan).value.is_nil() && (*scan).key == key {
                        // Key exists alive in chain — just update value
                        (*scan).value = value;
                        return;
                    }
                }
                // Key not found in chain — safe to reuse this dead slot
                (*mp).key = key;
                (*mp).value = value;
                // Keep (*mp).next as-is — other chains may pass through here
                return;
            }

            // Main position is occupied by a LIVE entry.
            // Check if the occupying node belongs here.
            // Port of C Lua 5.5 newkey collision handling.
            let othern = self.mainposition(&(*mp).key);

            if othern != mp {
                // Case 1: Colliding node is NOT at its main position (displaced).
                // Move the displaced node to a free slot and give mp to the new key.
                // First, get a free position.
                if let Some(free_node) = self.getfreepos() {
                    // Find the previous node in the displaced node's own chain
                    let mut prev = othern;
                    while prev.offset((*prev).next as isize) != mp {
                        prev = prev.offset((*prev).next as isize);
                    }
                    // Relink previous to point to free_node instead of mp
                    (*prev).next = Self::node_offset(prev, free_node);
                    // Copy the displaced node into the free slot (including its next pointer)
                    *free_node = *mp;
                    // Correct the next pointer: it was relative to mp, now it's relative to free_node
                    if (*free_node).next != 0 {
                        (*free_node).next += Self::node_offset(free_node, mp);
                    }
                    // Now mp is free for the new key
                    (*mp).key = key;
                    (*mp).value = value;
                    (*mp).next = 0;
                    return;
                }
                // No free position - fall through to resize
            } else {
                // Case 2: Colliding node IS at its main position.
                // The new key will go into a free slot, linked into mp's chain.

                // First check if key already exists in the chain
                let mut node = mp;
                loop {
                    // Skip dead keys (value nil) to avoid UAF on freed strings
                    if !(*node).value.is_nil() && (*node).key == key {
                        (*node).value = value;
                        return;
                    }
                    let next = (*node).next;
                    if next == 0 {
                        break;
                    }
                    node = node.offset(next as isize);
                }

                // Key not found - insert at a free position
                if let Some(free_node) = self.getfreepos() {
                    (*free_node).key = key;
                    (*free_node).value = value;
                    // Link new node at the HEAD of the chain (right after mp),
                    // matching C Lua 5.5 behavior: gnext(f) = mp+gnext(mp) - f
                    if (*mp).next != 0 {
                        (*free_node).next =
                            Self::node_offset(free_node, mp.offset((*mp).next as isize));
                    } else {
                        (*free_node).next = 0;
                    }
                    (*mp).next = Self::node_offset(mp, free_node);
                    return;
                }
                // No free position - fall through to resize
            }

            // No free nodes - need to rehash (Lua 5.5's rehash)
            // This rebalances integer keys between array and hash parts,
            // which is critical when GC has cleared weak table entries
            // and corrupted lenhint, causing integer keys to end up in hash.
            self.rehash(&key);
            // After rehash, use raw_set to insert with the new layout
            self.raw_set(&key, value);
        }
    }

    // Delete a key from hash table
    // fn delete_node(&mut self, key: &LuaValue) {
    //     if self.sizenode() == 0 {
    //         return;
    //     }

    //     unsafe {
    //         let mp = self.mainposition(key);
    //         let mut node = mp;

    //         // Find the node with this key
    //         loop {
    //             if (*node).key == *key {
    //                 // Found it - mark as deleted by setting key to nil
    //                 (*node).key = LuaValue::nil();
    //                 (*node).value = LuaValue::nil();
    //                 // Note: We keep the chain intact (next field) for iteration
    //                 return;
    //             }

    //             let next = (*node).next;
    //             if next == 0 {
    //                 // Key not found
    //                 return;
    //             }
    //             node = node.offset(next as isize);
    //         }
    //     }
    // }

    /// Generic set - returns true if new key was inserted
    #[inline(always)]
    /// Port of lua5.5's newcheckedkey logic in luaH_set/luaH_setint
    /// CRITICAL INVARIANT: integer keys in [1..asize] must ONLY exist in array part!
    pub fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> bool {
        // Normalize float keys to integer if they have no fractional part
        // This ensures t[3.0] and t[3] refer to the same slot
        // Only check actual float type (ttisfloat), not integers
        let mut key = *key;
        if key.ttisfloat() {
            let f = key.fltvalue();
            if f.fract() == 0.0 && f.is_finite() {
                let i = f as i64;
                if i as f64 == f {
                    key = LuaValue::integer(i);
                }
            }
        };

        // Check if key is an integer in array range (lua5.5's keyinarray check)
        if key.ttisinteger() {
            let i = key.ivalue();
            if i >= 1 && i <= self.asize as i64 {
                // Key is in array range - set in array part ONLY
                let was_nil = unsafe { self.read_array(i).is_none() };
                unsafe {
                    self.write_array(i, value);
                }
                // DEFENSIVE: If setting to nil, also clear any stale hash entry
                // for this integer key. This can happen when GC clearing corrupted
                // lenhint and a key was placed in both array and hash parts.
                if value.is_nil() {
                    self.set_node(key, LuaValue::nil());
                }
                return was_nil && !value.is_nil();
            }

            // Integer key outside current array range
            // Use Lua 5.5-like approach: if this is a sequential push (i == len+1),
            // expand array optimistically. After expanding, migrate any integer
            // keys from hash into the new array range to maintain the invariant
            // that integer keys in [1..asize] only exist in the array part.
            if i >= 1 && !value.is_nil() {
                let current_len = self.len() as i64;
                if i == current_len + 1 {
                    let new_size = ((i as u32).next_power_of_two()).max(4);
                    self.resize_array(new_size);
                    unsafe {
                        self.write_array(i, value);
                    }
                    self.migrate_hash_int_keys_to_array();
                    return true;
                }
            }
        }

        // Not in array range - use hash part
        // lua5.5's insertkey/newcheckedkey logic
        let key_exists = self.get_from_hash(&key).is_some();
        self.set_node(key, value);
        !key_exists && !value.is_nil()
    }

    /// Get length (#t) — Lua 5.5 boundary algorithm (luaH_getn).
    ///
    /// A "boundary" is an integer index i such that t[i] is present and t[i+1]
    /// is absent, or 0 if t[1] is absent.
    ///
    /// Uses lenhint as a starting hint, searches the vicinity first,
    /// then falls back to binary search. If the array's last element is
    /// non-empty, also searches the hash part for integer-key continuation.
    pub fn len(&self) -> usize {
        let asize = self.asize;
        if asize > 0 && !self.array.is_null() {
            const MAX_VICINITY: u32 = 4;
            let mut limit = unsafe { *self.lenhint_ptr() };
            if limit == 0 {
                limit = 1; // make limit a valid array index
            }

            if self.array_key_is_empty(limit) {
                // t[limit] is empty — border must be before limit
                // Look in the vicinity first
                let mut i = 0;
                while i < MAX_VICINITY && limit > 1 {
                    limit -= 1;
                    if !self.array_key_is_empty(limit) {
                        return self.newhint(limit) as usize;
                    }
                    i += 1;
                }
                // Still empty — binary search in [0, limit)
                return self.newhint(self.binsearch(0, limit)) as usize;
            } else {
                // t[limit] is present — look for border after it
                let mut i = 0;
                while i < MAX_VICINITY && limit < asize {
                    limit += 1;
                    if self.array_key_is_empty(limit) {
                        return self.newhint(limit - 1) as usize;
                    }
                    i += 1;
                }
                if self.array_key_is_empty(asize) {
                    // t[limit] not empty but t[asize] empty — binary search
                    return self.newhint(self.binsearch(limit, asize)) as usize;
                }
            }
            // Last array element is non-empty — set hint to asize
            unsafe {
                *self.lenhint_ptr() = asize;
            }
        }
        // No array part, or t[asize] is not empty — check hash part
        debug_assert!(asize == 0 || self.array.is_null() || !self.array_key_is_empty(asize));
        if self.is_dummy() || self.hash_key_is_empty(asize as u64 + 1) {
            return asize as usize; // asize + 1 is empty
        }
        // asize + 1 is also non-empty — search hash part
        self.hash_search(asize) as usize
    }

    /// Check if array key (1-based) is empty (nil or uninitialized)
    #[inline(always)]
    fn array_key_is_empty(&self, key: u32) -> bool {
        if key < 1 || key > self.asize {
            return true;
        }
        unsafe {
            let tag = *self.get_arr_tag((key - 1) as usize);
            tag == LUA_VNIL || tag == LUA_VEMPTY
        }
    }

    /// Check if an integer key is absent from the hash part
    #[inline(always)]
    fn hash_key_is_empty(&self, key: u64) -> bool {
        if self.is_dummy() {
            return true;
        }
        let lookup_key = LuaValue::integer(key as i64);
        self.get_from_hash(&lookup_key).is_none()
    }

    /// Binary search for a boundary in the array part.
    /// Precondition: t[i] is present (or i==0), t[j] is absent.
    /// Returns a boundary index.
    fn binsearch(&self, mut i: u32, mut j: u32) -> u32 {
        debug_assert!(i <= j);
        while j - i > 1 {
            let m = (i + j) / 2;
            if self.array_key_is_empty(m) {
                j = m;
            } else {
                i = m;
            }
        }
        i
    }

    /// Search the hash part for a boundary, knowing that t[asize+1] is present
    /// in the hash. Uses doubling + binary search.
    fn hash_search(&self, asize: u32) -> u64 {
        let mut i: u64 = asize as u64 + 1; // caller ensures t[i] is present
        let mut j: u64 = i * 2;

        // Find an absent key by doubling
        while !self.hash_key_is_empty(j) {
            i = j;
            if j < u64::MAX / 2 {
                j *= 2;
            } else {
                j = i64::MAX as u64;
                if self.hash_key_is_empty(j) {
                    break;
                } else {
                    return j; // max integer is a boundary
                }
            }
        }
        // Binary search between i (present) and j (absent)
        while j - i > 1 {
            let m = (i + j) / 2;
            if self.hash_key_is_empty(m) {
                j = m;
            } else {
                i = m;
            }
        }
        i
    }

    /// Save a new hint and return it
    #[inline(always)]
    fn newhint(&self, hint: u32) -> u32 {
        debug_assert!(hint <= self.asize);
        if !self.array.is_null() {
            unsafe {
                *self.lenhint_ptr() = hint;
            }
        }
        hint
    }

    /// Get hash size
    #[inline(always)]
    pub fn hash_size(&self) -> usize {
        self.sizenode()
    }

    /// Remove value at lua_index (1-based), shifting elements backward
    /// This is the efficient implementation for table.remove(t, pos)
    pub fn remove_at(&mut self, lua_index: i64) -> Option<LuaValue> {
        if lua_index < 1 {
            return None;
        }

        let len = self.len() as i64;

        if lua_index > len {
            return None;
        }

        // Get the value to return
        let value = unsafe { self.read_array(lua_index)? };

        // Shift elements from lua_index+1 to len backward by 1
        unsafe {
            for j in lua_index..len {
                // Always read and shift, even if nil
                let k_next = j as usize;
                let tt = *self.get_arr_tag(k_next);
                let val_ptr = self.get_arr_val(k_next);
                let val = *val_ptr;

                // Write to current position
                let k = (j - 1) as usize;
                *self.get_arr_tag(k) = tt;
                *self.get_arr_val(k) = val;
            }

            // Clear the last position
            self.write_array(len, LuaValue::nil());

            // Update lenhint - length decreased by 1
            let new_len = (len - 1) as u32;
            *self.lenhint_ptr() = new_len;
        }

        Some(value)
    }

    /// Iterate to next key-value pair
    /// Port of lua5.5's findindex
    /// Returns the unified index for table traversal:
    /// - 0 for nil (first iteration)
    /// - 1..asize for array indices
    /// - (asize+1)..(asize+hashsize) for hash indices
    fn findindex(&self, key: &LuaValue) -> Option<u32> {
        // First iteration
        if key.is_nil() {
            return Some(0);
        }

        // Check if key is in array part (Lua 5.5's keyinarray).
        // Only checks if integer key is in [1..asize] — does NOT check whether
        // the slot is empty. This is critical for next() iteration: after setting
        // t[k] = nil during pairs(), the slot is empty but findindex must still
        // return the index so iteration can continue past deleted entries.
        if let Some(i) = key.as_integer()
            && i >= 1
            && i <= self.asize as i64
        {
            return Some(i as u32);
        }

        // Key must be in hash part - search for it (Lua 5.5's getgeneric with deadok=1)
        let size = self.sizenode();
        if size == 0 {
            return None; // No hash part, key not found
        }

        let main_pos = self.mainposition(key);
        let mut node = main_pos;

        unsafe {
            loop {
                // Check live keys first (value not nil): full equality comparison
                if !(*node).value.is_nil() && (*node).key == *key {
                    let hash_idx =
                        (node as usize - self.node as usize) / std::mem::size_of::<Node>();
                    return Some((hash_idx as u32 + 1) + self.asize);
                }
                // Check dead keys (value is nil, key preserved): use RAW comparison
                // only (type tag + raw value bits). This avoids dereferencing freed
                // GC objects (e.g., long strings) while still matching the exact
                // same object returned by a previous next() call.
                // Matches Lua 5.5's equalkey with deadok=1, which uses pointer
                // comparison (gcvalue(k1) == gcvalueraw(keyval(n2))) for dead keys.
                if (*node).value.is_nil()
                    && !(*node).key.is_nil()
                    && (*node).key.tt == key.tt
                    && (*node).key.value.i == key.value.i
                {
                    let hash_idx =
                        (node as usize - self.node as usize) / std::mem::size_of::<Node>();
                    return Some((hash_idx as u32 + 1) + self.asize);
                }

                let next_offset = (*node).next;
                if next_offset == 0 {
                    return None; // Key not found in chain
                }
                node = node.offset(next_offset as isize);
            }
        }
    }

    /// Port of lua5.5's luaH_next
    /// Table iteration following the unified indexing scheme
    /// Port of lua5.5's luaH_next.
    /// Returns Ok(Some((key, value))) for next entry, Ok(None) for end of table,
    /// or Err(()) for invalid key (key not found in table).
    pub fn next(&self, key: &LuaValue) -> Result<Option<(LuaValue, LuaValue)>, ()> {
        let asize = self.asize;

        // Get starting index from the input key
        let mut i = match self.findindex(key) {
            Some(idx) => idx,
            None => return Err(()), // Invalid key (not found in table)
        };

        // First, scan the array part [i..asize)
        while i < asize {
            unsafe {
                let tag = *self.get_arr_tag(i as usize);
                if tag != LUA_VNIL && tag != LUA_VEMPTY {
                    // Found a non-empty array entry
                    let lua_index = (i + 1) as i64;
                    let value = self.read_array(lua_index).unwrap();
                    return Ok(Some((LuaValue::integer(lua_index), value)));
                }
            }
            i += 1;
        }

        // Array exhausted, now scan hash part
        let hash_size = self.sizenode() as u32;
        i -= asize; // Convert unified index to hash index

        while i < hash_size {
            unsafe {
                let node = self.node.add(i as usize);
                if !(*node).key.is_nil() && !(*node).value.is_nil() {
                    // Found a non-empty hash entry (key present and value not nil)
                    return Ok(Some(((*node).key, (*node).value)));
                }
            }
            i += 1;
        }

        Ok(None) // No more elements
    }

    /// GC-safe iteration: call f for each entry
    pub fn for_each_entry<F>(&self, mut f: F)
    where
        F: FnMut(LuaValue, LuaValue),
    {
        // Iterate array part
        for i in 1..=self.asize as i64 {
            unsafe {
                if let Some(val) = self.read_array(i) {
                    f(LuaValue::integer(i), val);
                }
            }
        }

        // Iterate hash part
        let size = self.sizenode();
        for i in 0..size {
            unsafe {
                let node = self.node.add(i);
                if !(*node).key.is_nil() && !(*node).value.is_nil() {
                    f((*node).key, (*node).value);
                }
            }
        }
    }
}

impl Drop for NativeTable {
    fn drop(&mut self) {
        // Free array - must deallocate from start pointer, not array pointer
        if !self.array.is_null() && self.asize > 0 {
            let values_size = self.asize as usize * std::mem::size_of::<Value>();
            let lenhint_size = std::mem::size_of::<u32>();
            let tags_size = self.asize as usize;
            let total_size = values_size + lenhint_size + tags_size;

            // array points to lenhint, so start is array - values_size
            let start_ptr = unsafe { self.array.sub(values_size) };
            let layout =
                Layout::from_size_align(total_size, std::mem::align_of::<Value>()).unwrap();
            unsafe { alloc::dealloc(start_ptr, layout) };
        }

        // Free hash
        let size = self.sizenode();
        if size > 0 && !self.is_dummy() {
            let layout = Layout::array::<Node>(size).unwrap();
            unsafe { alloc::dealloc(self.node as *mut u8, layout) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_table_basic() {
        let mut t = NativeTable::new(4, 4);

        // Test integer keys
        let key1 = LuaValue::integer(1);
        let val1 = LuaValue::integer(100);
        t.raw_set(&key1, val1);

        assert_eq!(t.raw_get(&key1), Some(val1));

        // Test more integer keys
        for i in 1..=10 {
            t.set_int(i, LuaValue::integer(i * 10));
        }

        for i in 1..=10 {
            assert_eq!(t.get_int(i), Some(LuaValue::integer(i * 10)));
        }
    }

    #[test]
    fn test_array_part() {
        let mut t = NativeTable::new(10, 0);

        for i in 1..=10 {
            t.set_int(i, LuaValue::integer(i * 10));
        }

        for i in 1..=10 {
            assert_eq!(t.get_int(i), Some(LuaValue::integer(i * 10)));
        }

        assert_eq!(t.len(), 10);
    }

    #[test]
    fn test_hash_collisions() {
        let mut t = NativeTable::new(0, 4);

        // Add many items to force collisions
        for i in 0..20 {
            let key = LuaValue::integer(i);
            let val = LuaValue::integer(i * 100);
            t.raw_set(&key, val);
        }

        // Verify all items
        for i in 0..20 {
            let key = LuaValue::integer(i);
            let expected = LuaValue::integer(i * 100);
            assert_eq!(t.raw_get(&key), Some(expected), "Failed for key {}", i);
        }
    }

    #[test]
    fn test_performance_integer_keys() {
        use std::time::Instant;

        let mut t = NativeTable::new(100, 100);

        let start = Instant::now();

        // Insert
        for i in 0..10000 {
            t.set_int(i, LuaValue::integer(i));
        }

        // Read
        for i in 0..10000 {
            let val = t.get_int(i);
            assert_eq!(val, Some(LuaValue::integer(i)));
        }

        let elapsed = start.elapsed();
        println!("NativeTable integer ops (20k ops): {:?}", elapsed);
        println!("Per-op: {:?}", elapsed / 20000);
    }
}
