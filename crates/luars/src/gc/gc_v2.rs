// Lua-style Garbage Collector V2
// 
// Key Design Changes from V1:
// 1. NO HashMap for object tracking - objects are already in Arena
// 2. GC header embedded in objects (already done in ObjectPool)
// 3. Mark phase traverses from roots using Arena iteration
// 4. Sweep phase iterates Arena directly
// 5. Write barrier only checks color bits, no HashMap lookup
//
// This eliminates the main performance bottleneck: HashMap operations on every allocation

use crate::gc::{ObjectPool, GcObjectType};
use crate::lua_value::LuaValue;

/// Lua 5.4 style GC colors
/// Using a single byte for color + age bits (like Lua's marked field)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum GcColor {
    /// White: not yet visited (will be collected if not marked)
    White0 = 0,
    /// White (alternate): for double-buffering white bits
    White1 = 1,
    /// Gray: marked but children not yet scanned
    Gray = 2,
    /// Black: marked and all children scanned
    Black = 3,
}

/// Lightweight GC state
/// No per-object tracking - just global state for the GC cycle
pub struct GcV2 {
    // Current white color (alternates between White0 and White1)
    current_white: GcColor,
    
    // GC debt mechanism (Lua 5.4 style)
    // Positive debt triggers GC step
    pub gc_debt: isize,
    
    // Total bytes allocated (for statistics)
    pub total_bytes: usize,
    
    // GC estimate (non-garbage memory)
    gc_estimate: usize,
    
    // GC pause parameter (percentage, default 200 = wait for memory to double)
    gc_pause: usize,
    
    // Statistics
    pub collection_count: usize,
    pub bytes_freed: usize,
}

impl GcV2 {
    pub fn new() -> Self {
        Self {
            current_white: GcColor::White0,
            gc_debt: -(200 * 1024), // Start with negative debt (200KB before first GC)
            total_bytes: 0,
            gc_estimate: 0,
            gc_pause: 200,
            collection_count: 0,
            bytes_freed: 0,
        }
    }

    /// Record an allocation - just update debt, O(1)
    #[inline(always)]
    pub fn record_allocation(&mut self, size: usize) {
        self.total_bytes += size;
        self.gc_debt += size as isize;
    }

    /// Check if GC should run - single comparison, O(1)
    #[inline(always)]
    pub fn should_collect(&self) -> bool {
        self.gc_debt > 0
    }

    /// Get the "other" white color (for checking dead objects)
    #[inline(always)]
    fn other_white(&self) -> GcColor {
        match self.current_white {
            GcColor::White0 => GcColor::White1,
            GcColor::White1 => GcColor::White0,
            _ => unreachable!(),
        }
    }

    /// Full mark-sweep collection
    /// Called when gc_debt > 0
    pub fn collect(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) -> usize {
        self.collection_count += 1;
        
        // 1. Clear all marks (set to current white)
        self.clear_marks(pool);
        
        // 2. Mark phase: traverse from roots
        let mut gray_list = Vec::with_capacity(64);
        for root in roots {
            self.mark_value(root, pool, &mut gray_list);
        }
        
        // Process gray objects until empty
        while let Some(value) = gray_list.pop() {
            self.traverse_value(&value, pool, &mut gray_list);
        }
        
        // 3. Sweep phase: free unmarked objects
        let freed = self.sweep(pool);
        
        // 4. Flip white color for next cycle
        self.current_white = self.other_white();
        
        // 5. Update debt based on surviving memory
        self.set_debt();
        
        self.bytes_freed += freed;
        freed
    }

    /// Clear all marks in pool
    fn clear_marks(&self, pool: &mut ObjectPool) {
        // Tables
        for (_, table) in pool.tables.iter_mut() {
            table.header.marked = false;
        }
        // Strings  
        for (_, string) in pool.strings.iter_mut() {
            string.header.marked = false;
        }
        // Functions
        for (_, func) in pool.functions.iter_mut() {
            func.header.marked = false;
        }
        // Upvalues
        for (_, uv) in pool.upvalues.iter_mut() {
            uv.header.marked = false;
        }
        // Threads
        for (_, thread) in pool.threads.iter_mut() {
            thread.header.marked = false;
        }
    }

    /// Mark a value (add to gray list if not already marked)
    #[inline]
    fn mark_value(&self, value: &LuaValue, pool: &ObjectPool, gray: &mut Vec<LuaValue>) {
        match value.kind() {
            crate::lua_value::LuaValueKind::Table => {
                if let Some(id) = value.as_table_id() {
                    if let Some(table) = pool.tables.get(id.0) {
                        if !table.header.marked {
                            gray.push(*value);
                        }
                    }
                }
            }
            crate::lua_value::LuaValueKind::String => {
                if let Some(id) = value.as_string_id() {
                    if let Some(string) = pool.strings.get(id.0) {
                        if !string.header.marked {
                            // Strings have no children, mark directly
                            // Note: we need mutable access, handled in traverse
                            gray.push(*value);
                        }
                    }
                }
            }
            crate::lua_value::LuaValueKind::Function => {
                if let Some(id) = value.as_function_id() {
                    if let Some(func) = pool.functions.get(id.0) {
                        if !func.header.marked {
                            gray.push(*value);
                        }
                    }
                }
            }
            _ => {} // Non-GC types
        }
    }

    /// Traverse a gray object (mark it black and mark its children)
    fn traverse_value(&self, value: &LuaValue, pool: &mut ObjectPool, gray: &mut Vec<LuaValue>) {
        match value.kind() {
            crate::lua_value::LuaValueKind::Table => {
                if let Some(id) = value.as_table_id() {
                    if let Some(table) = pool.tables.get_mut(id.0) {
                        if table.header.marked {
                            return; // Already processed
                        }
                        table.header.marked = true;
                        
                        // Mark metatable
                        if let Some(mt) = table.data.get_metatable() {
                            self.mark_value(&mt, pool, gray);
                        }
                        
                        // Mark array elements
                        for val in table.data.array_iter() {
                            self.mark_value(val, pool, gray);
                        }
                        
                        // Mark hash elements
                        for (key, val) in table.data.hash_iter() {
                            self.mark_value(key, pool, gray);
                            self.mark_value(val, pool, gray);
                        }
                    }
                }
            }
            crate::lua_value::LuaValueKind::String => {
                if let Some(id) = value.as_string_id() {
                    if let Some(string) = pool.strings.get_mut(id.0) {
                        string.header.marked = true;
                        // Strings have no children
                    }
                }
            }
            crate::lua_value::LuaValueKind::Function => {
                if let Some(id) = value.as_function_id() {
                    if let Some(func) = pool.functions.get_mut(id.0) {
                        if func.header.marked {
                            return;
                        }
                        func.header.marked = true;
                        
                        // Mark upvalues
                        for uv_id in &func.upvalues.clone() {
                            if let Some(uv) = pool.upvalues.get_mut(uv_id.0) {
                                if !uv.header.marked {
                                    uv.header.marked = true;
                                    if let Some(val) = uv.get_closed_value() {
                                        self.mark_value(&val, pool, gray);
                                    }
                                }
                            }
                        }
                        
                        // Mark constants in chunk
                        for constant in &func.chunk.constants {
                            self.mark_value(constant, pool, gray);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Sweep: free all unmarked objects
    fn sweep(&mut self, pool: &mut ObjectPool) -> usize {
        let mut freed = 0;
        
        // Collect IDs of unmarked objects
        let tables_to_free: Vec<u32> = pool.tables.iter()
            .filter(|(_, t)| !t.header.marked)
            .map(|(id, _)| id)
            .collect();
            
        let strings_to_free: Vec<u32> = pool.strings.iter()
            .filter(|(_, s)| !s.header.marked)
            .map(|(id, _)| id)
            .collect();
            
        let functions_to_free: Vec<u32> = pool.functions.iter()
            .filter(|(_, f)| !f.header.marked)
            .map(|(id, _)| id)
            .collect();
            
        let upvalues_to_free: Vec<u32> = pool.upvalues.iter()
            .filter(|(_, u)| !u.header.marked)
            .map(|(id, _)| id)
            .collect();
            
        let threads_to_free: Vec<u32> = pool.threads.iter()
            .filter(|(_, t)| !t.header.marked)
            .map(|(id, _)| id)
            .collect();

        // Free objects
        for id in tables_to_free {
            pool.tables.free(id);
            freed += 256; // Estimated table size
            self.total_bytes = self.total_bytes.saturating_sub(256);
        }
        
        for id in strings_to_free {
            // Remove from intern table if needed
            // ... (handled by ObjectPool)
            pool.strings.free(id);
            freed += 64;
            self.total_bytes = self.total_bytes.saturating_sub(64);
        }
        
        for id in functions_to_free {
            pool.functions.free(id);
            freed += 128;
            self.total_bytes = self.total_bytes.saturating_sub(128);
        }
        
        for id in upvalues_to_free {
            pool.upvalues.free(id);
            freed += 32;
            self.total_bytes = self.total_bytes.saturating_sub(32);
        }
        
        for id in threads_to_free {
            pool.threads.free(id);
            freed += 512;
            self.total_bytes = self.total_bytes.saturating_sub(512);
        }

        freed
    }

    /// Set debt based on current memory and pause parameter
    fn set_debt(&mut self) {
        let estimate = self.total_bytes.max(1024);
        // debt = -(estimate * pause / 100)
        // This means we can allocate `estimate * pause / 100` bytes before next GC
        let pause_bytes = (estimate * self.gc_pause) / 100;
        self.gc_debt = -(pause_bytes as isize);
        self.gc_estimate = estimate;
    }
}

impl Default for GcV2 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_debt() {
        let mut gc = GcV2::new();
        assert!(!gc.should_collect()); // Starts with negative debt
        
        // Allocate until debt is positive
        for _ in 0..1000 {
            gc.record_allocation(256);
        }
        
        assert!(gc.should_collect());
    }
}
