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
#[repr(C)]
struct Node {
    /// Value stored in this node
    value: LuaValue,
    /// Key stored in this node  
    key: LuaValue,
    /// Next node in collision chain (offset, 0 = end)
    next: i32,
}

/// Dummy node for empty hash tables
const DUMMY_NODE: Node = Node {
    value: LuaValue {
        value: Value { i: 0 },
        tt: LUA_VNIL,
    },
    key: LuaValue {
        value: Value { i: 0 },
        tt: LUA_VNIL,
    },
    next: 0,
};

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
        if self.node.is_null() || self.node == &DUMMY_NODE as *const Node as *mut Node {
            0
        } else {
            1usize << self.lsizenode
        }
    }

    #[inline(always)]
    fn is_dummy(&self) -> bool {
        self.node.is_null() || self.node == &DUMMY_NODE as *const Node as *mut Node
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
    unsafe fn write_array(&mut self, lua_index: i64, luaval: LuaValue) {
        if lua_index < 1 || lua_index > self.asize as i64 {
            return;
        }
        let k = (lua_index - 1) as usize; // Convert to 0-based C index

        unsafe {
            *self.get_arr_tag(k) = luaval.tt;
            *self.get_arr_val(k) = luaval.value;

            // Update lenhint
            let lenhint = *self.lenhint_ptr();

            if !luaval.is_nil() {
                // Adding a non-nil value
                if lua_index == lenhint as i64 + 1 {
                    // Extending the array
                    *self.lenhint_ptr() = lenhint + 1;
                } else if lua_index > lenhint as i64 + 1 {
                    // Adding beyond lenhint - lenhint stays the same (there's a hole)
                }
            } else {
                // Setting to nil
                // Only reduce lenhint if we're clearing the last element
                // For holes in the middle, pairs() will skip them correctly
                if lua_index == lenhint as i64 {
                    // Find the new lenhint by scanning backwards
                    let mut new_lenhint = lua_index as u32 - 1;
                    while new_lenhint > 0 {
                        let check_idx = new_lenhint as usize - 1;
                        let tag = *self.get_arr_tag(check_idx);
                        if tag != LUA_VNIL && tag != LUA_VEMPTY {
                            break;
                        }
                        new_lenhint -= 1;
                    }
                    *self.lenhint_ptr() = new_lenhint;
                }
                // If clearing an element in the middle, lenhint stays the same
                // This allows pairs() to continue iterating past holes
            }
        }
    }

    /// Resize array part
    fn resize_array(&mut self, new_size: u32) {
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

        // Initialize lenhint
        unsafe {
            *(new_array as *mut u32) = 0;
        }

        // Initialize all tags to nil
        unsafe {
            let tags_start = new_array.add(lenhint_size);
            for i in 0..new_size as usize {
                *tags_start.add(i) = LUA_VNIL;
            }
        }

        // Initialize all values to zero
        unsafe {
            let values_start = start_ptr as *mut Value;
            for i in 0..new_size as usize {
                ptr::write(values_start.add(i), Value { i: 0 });
            }
        }

        // Copy old data if exists
        if !self.array.is_null() && old_size > 0 {
            let copy_size = old_size.min(new_size) as usize;

            unsafe {
                // Copy values - values are stored backward from array pointer
                // Old: array_old - old_size*8 .. array_old
                // New: array_new - new_size*8 .. array_new
                // We need to copy the old values to the END of the new values section
                let old_values_start = self
                    .array
                    .sub(old_size as usize * std::mem::size_of::<Value>())
                    as *const Value;
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
        if !was_dummy && old_size > 0 {
            for i in 0..old_size {
                unsafe {
                    let old_n = old_node.add(i);
                    if !(*old_n).key.is_nil() {
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

    /// Set value in array part
    #[inline(always)]
    pub fn set_int(&mut self, key: i64, value: LuaValue) {
        // If key is in valid array range
        if key >= 1 && key <= self.asize as i64 {
            unsafe {
                self.write_array(key, value);
            }
            return;
        }

        // Only expand array if key is exactly length+1 (push operation)
        // This avoids creating sparse arrays with large holes
        if key >= 1 {
            let current_len = self.len() as i64;
            if key == current_len + 1 {
                // This is a push operation, expand array
                let new_size = ((key as u32).next_power_of_two()).max(4);
                self.resize_array(new_size);
                unsafe {
                    self.write_array(key, value);
                }
                return;
            }
        }

        // Put in hash part
        let key_val = LuaValue::integer(key);
        self.set_node(key_val, value);
    }

    /// Get value from hash part - CRITICAL HOT PATH
    #[inline(always)]
    fn get_from_hash(&self, key: &LuaValue) -> Option<LuaValue> {
        if self.sizenode() == 0 {
            return None;
        }

        // Fast path for short strings only - direct pointer comparison
        // Long strings (>40 chars) are NOT interned, so must use general case
        if key.is_short_string() {
            return self.get_shortstr_fast(key);
        }

        // General case (includes long strings)
        let mut node = self.mainposition(key);

        loop {
            unsafe {
                // Compare keys with proper equality (handles long string content comparison)
                if (*node).key == *key {
                    let val = (*node).value;
                    return if val.is_nil() { None } else { Some(val) };
                }

                let next = (*node).next;
                if next == 0 {
                    return None;
                }
                node = node.offset(next as isize);
            }
        }
    }

    /// Fast path for short string lookup - mimics luaH_Hgetshortstr
    /// OPTIMIZED: Reduced branches in hot loop
    #[inline(always)]
    fn get_shortstr_fast(&self, key: &LuaValue) -> Option<LuaValue> {
        let mut node = self.mainposition(key);
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
        // Try array part for integers
        if let Some(i) = key.as_integer() {
            unsafe {
                if let Some(val) = self.read_array(i) {
                    return Some(val);
                }
            }
        }

        // Hash part
        self.get_from_hash(key)
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

    fn set_node(&mut self, key: LuaValue, value: LuaValue) {
        // If setting to nil, we should delete the key
        if value.is_nil() {
            self.delete_node(&key);
            return;
        }

        if self.sizenode() == 0 {
            // Need to allocate hash part
            self.resize_hash(2); // Start with 4 nodes
        }

        let mp = self.mainposition(&key);

        unsafe {
            // If main position is free, use it
            if (*mp).key.is_nil() {
                (*mp).key = key;
                (*mp).value = value;
                (*mp).next = 0;
                return;
            }

            // Check if key already exists
            let mut node = mp;
            loop {
                if (*node).key == key {
                    (*node).value = value;
                    return;
                }

                let next = (*node).next;
                if next == 0 {
                    break;
                }
                node = node.offset(next as isize);
            }

            // Need to add new node - find free position using getfreepos
            if let Some(free_node) = self.getfreepos() {
                // Found free node
                (*free_node).key = key;
                (*free_node).value = value;
                (*free_node).next = 0;

                // Link to chain
                (*node).next = (free_node as isize - node as isize) as i32
                    / std::mem::size_of::<Node>() as i32;
                return;
            }

            // No free nodes - need to resize
            self.resize_hash(self.lsizenode + 1);
            self.set_node(key, value);
        }
    }

    /// Delete a key from hash table
    fn delete_node(&mut self, key: &LuaValue) {
        if self.sizenode() == 0 {
            return;
        }

        unsafe {
            let mp = self.mainposition(key);
            let mut node = mp;

            // Find the node with this key
            loop {
                if (*node).key == *key {
                    // Found it - mark as deleted by setting key to nil
                    (*node).key = LuaValue::nil();
                    (*node).value = LuaValue::nil();
                    // Note: We keep the chain intact (next field) for iteration
                    return;
                }

                let next = (*node).next;
                if next == 0 {
                    // Key not found
                    return;
                }
                node = node.offset(next as isize);
            }
        }
    }

    /// Generic set - returns true if new key was inserted
    #[inline(always)]
    /// Port of lua5.5's newcheckedkey logic in luaH_set/luaH_setint
    /// CRITICAL INVARIANT: integer keys in [1..asize] must ONLY exist in array part!
    pub fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> bool {
        // Check if key is an integer in array range (lua5.5's keyinarray check)
        if let Some(i) = key.as_integer() {
            if i >= 1 && i <= self.asize as i64 {
                // Key is in array range - set in array part ONLY
                let was_nil = unsafe { self.read_array(i).is_none() };
                unsafe {
                    self.write_array(i, value);
                }
                return was_nil && !value.is_nil();
            }

            // Integer key outside current array range
            // If it's a push operation (i == len+1), expand array
            if i >= 1 {
                let current_len = self.len() as i64;
                if i == current_len + 1 {
                    let new_size = ((i as u32).next_power_of_two()).max(4);
                    self.resize_array(new_size);
                    unsafe {
                        self.write_array(i, value);
                    }
                    return true;
                }
            }
        }

        // Not in array range - use hash part
        // lua5.5's insertkey/newcheckedkey logic
        let key_exists = self.get_from_hash(key).is_some();
        self.set_node(*key, value);
        !key_exists && !value.is_nil()
    }

    /// Get length (#t)
    #[inline(always)]
    pub fn len(&self) -> usize {
        if self.array.is_null() {
            return 0;
        }
        unsafe { *self.lenhint_ptr() as usize }
    }

    /// Get hash size
    #[inline(always)]
    pub fn hash_size(&self) -> usize {
        self.sizenode()
    }

    /// Insert value at lua_index (1-based), shifting elements forward
    /// This is the efficient implementation for table.insert(t, pos, value)
    pub fn insert_at(&mut self, lua_index: i64, value: LuaValue) -> bool {
        if lua_index < 1 {
            return false;
        }

        let len = self.len() as i64;

        // If inserting beyond current length, just set it
        if lua_index > len {
            self.set_int(lua_index, value);
            return true;
        }

        // Need to shift elements - ensure array is large enough
        let needed_size = (len + 1) as u32;
        if needed_size > self.asize {
            let new_size = needed_size.next_power_of_two().max(4);
            self.resize_array(new_size);
        }

        // Shift elements from lua_index to len forward by 1
        unsafe {
            for j in (lua_index..=len).rev() {
                // Always read and shift, even if nil
                let k = (j - 1) as usize;
                let tt = *self.get_arr_tag(k);
                let val_ptr = self.get_arr_val(k);
                let val = *val_ptr;

                // Write to next position
                let k_next = j as usize;
                *self.get_arr_tag(k_next) = tt;
                *self.get_arr_val(k_next) = val;
            }

            // Insert at position
            self.write_array(lua_index, value);

            // Update lenhint if we extended the array
            if lua_index <= len {
                // We shifted elements, so length increased by 1
                let new_len = (len + 1) as u32;
                *self.lenhint_ptr() = new_len;
            }
        }

        true
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

        // Check if key is in array part (lua5.5's keyinarray)
        // For integer keys in [1..asize], return the index directly
        if let Some(i) = key.as_integer() {
            if i >= 1 && i <= self.asize as i64 {
                return Some(i as u32);
            }
        }

        // Key must be in hash part - search for it (lua5.5's getgeneric)
        let size = self.sizenode();
        if size == 0 {
            return None; // No hash part, key not found
        }

        let main_pos = self.mainposition(key);
        let mut node = main_pos;

        unsafe {
            loop {
                // Check if this node has our key
                if (*node).key == *key {
                    // Found the key, calculate its unified index
                    let hash_idx =
                        (node as usize - self.node as usize) / std::mem::size_of::<Node>();
                    return Some((hash_idx as u32 + 1) + self.asize);
                }

                let next_offset = (*node).next;
                if next_offset == 0 {
                    // Key not found in chain
                    return None;
                }
                node = node.offset(next_offset as isize);
            }
        }
    }

    /// Port of lua5.5's luaH_next
    /// Table iteration following the unified indexing scheme
    pub fn next(&self, key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        let asize = self.asize;

        // Get starting index from the input key
        let mut i = match self.findindex(key) {
            Some(idx) => idx,
            None => return None, // Invalid key (not found in table)
        };

        // First, scan the array part [i..asize)
        while i < asize {
            unsafe {
                let tag = *self.get_arr_tag(i as usize);
                if tag != LUA_VNIL && tag != LUA_VEMPTY {
                    // Found a non-empty array entry
                    let lua_index = (i + 1) as i64;
                    let value = self.read_array(lua_index).unwrap();
                    return Some((LuaValue::integer(lua_index), value));
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
                if !(*node).key.is_nil() {
                    // Found a non-empty hash entry
                    return Some(((*node).key, (*node).value));
                }
            }
            i += 1;
        }

        None // No more elements
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
                if !(*node).key.is_nil() {
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
