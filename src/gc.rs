// Garbage Collector for Lua VM
// Uses reference counting with cycle detection

use crate::value::LuaValue;
use std::collections::HashSet;
use std::rc::Rc;

/// Garbage collector state
pub struct GC {
    // Total bytes allocated
    bytes_allocated: usize,
    
    // GC threshold (trigger collection when exceeded)
    threshold: usize,
    
    // Statistics
    collection_count: usize,
}

impl GC {
    pub fn new() -> Self {
        GC {
            bytes_allocated: 0,
            threshold: 1024 * 1024, // 1MB initial threshold
            collection_count: 0,
        }
    }

    /// Check if GC should run
    pub fn should_collect(&self) -> bool {
        self.bytes_allocated > self.threshold
    }

    /// Perform garbage collection
    /// Takes root set (stack, globals, etc.) and marks reachable objects
    pub fn collect(&mut self, roots: &[LuaValue]) -> usize {
        self.collection_count += 1;
        
        // Mark phase: find all reachable objects
        let reachable = self.mark(roots);
        
        // Sweep phase would go here in a real GC
        // For now, we rely on Rc's automatic cleanup
        
        // Adjust threshold
        self.adjust_threshold();
        
        reachable.len()
    }

    /// Mark phase: traverse object graph from roots
    fn mark(&self, roots: &[LuaValue]) -> HashSet<usize> {
        let mut marked = HashSet::new();
        let mut worklist: Vec<LuaValue> = roots.to_vec();
        
        while let Some(value) = worklist.pop() {
            let id = self.value_id(&value);
            
            if marked.contains(&id) {
                continue;
            }
            
            marked.insert(id);
            
            // Mark children
            if let Some(table) = value.as_table() {
                self.mark_table(&table.borrow(), &mut worklist);
            } else if let Some(func) = value.as_function() {
                self.mark_function(&func, &mut worklist);
            }
        }
        
        marked
    }

    /// Mark table contents
    fn mark_table(&self, _table: &crate::value::LuaTable, _worklist: &mut Vec<LuaValue>) {
        // Note: This is simplified since we can't easily iterate internal HashMap
        // In a real implementation, we'd need to expose table internals to GC
    }

    /// Mark function upvalues
    fn mark_function(&self, func: &crate::value::LuaFunction, worklist: &mut Vec<LuaValue>) {
        for upval in &func.upvalues {
            worklist.push(upval.clone());
        }
    }

    /// Get unique ID for a value (for cycle detection)
    fn value_id(&self, value: &LuaValue) -> usize {
        if value.is_string() {
            value.as_string().map(|s| Rc::as_ptr(&s) as usize).unwrap_or(0)
        } else if value.is_table() {
            value.as_table().map(|t| Rc::as_ptr(&t) as usize).unwrap_or(0)
        } else if value.is_function() {
            value.as_function().map(|f| Rc::as_ptr(&f) as usize).unwrap_or(0)
        } else {
            0
        }
    }

    /// Adjust GC threshold based on current usage
    fn adjust_threshold(&mut self) {
        // Grow threshold based on current allocation
        self.threshold = (self.bytes_allocated * 2).max(1024 * 1024);
    }

    /// Record allocation
    pub fn record_allocation(&mut self, size: usize) {
        self.bytes_allocated += size;
    }

    /// Record deallocation
    pub fn record_deallocation(&mut self, size: usize) {
        self.bytes_allocated = self.bytes_allocated.saturating_sub(size);
    }

    /// Get statistics
    pub fn stats(&self) -> GCStats {
        GCStats {
            bytes_allocated: self.bytes_allocated,
            threshold: self.threshold,
            collection_count: self.collection_count,
        }
    }
}

/// GC statistics
#[derive(Debug, Clone)]
pub struct GCStats {
    pub bytes_allocated: usize,
    pub threshold: usize,
    pub collection_count: usize,
}

/// Memory pool for small object allocation
/// Reduces allocation overhead and improves cache locality
pub struct MemoryPool<T> {
    blocks: Vec<Vec<T>>,
    block_size: usize,
    free_list: Vec<*mut T>,
}

impl<T> MemoryPool<T> {
    pub fn new(block_size: usize) -> Self {
        MemoryPool {
            blocks: Vec::new(),
            block_size,
            free_list: Vec::new(),
        }
    }

    pub fn allocate(&mut self) -> *mut T 
    where
        T: Default,
    {
        if let Some(ptr) = self.free_list.pop() {
            return ptr;
        }

        // Allocate new block
        let mut block = Vec::with_capacity(self.block_size);
        for _ in 0..self.block_size {
            block.push(T::default());
        }

        let ptr = block.as_mut_ptr();
        self.blocks.push(block);

        // Add remaining slots to free list
        unsafe {
            for i in 1..self.block_size {
                self.free_list.push(ptr.add(i));
            }
        }

        ptr
    }

    pub fn deallocate(&mut self, ptr: *mut T) {
        self.free_list.push(ptr);
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.free_list.clear();
    }
}

unsafe impl<T> Send for MemoryPool<T> {}
unsafe impl<T> Sync for MemoryPool<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_creation() {
        let gc = GC::new();
        assert_eq!(gc.collection_count, 0);
        assert!(gc.bytes_allocated == 0);
    }

    #[test]
    fn test_gc_threshold() {
        let mut gc = GC::new();
        assert!(!gc.should_collect());
        
        gc.record_allocation(2 * 1024 * 1024);
        assert!(gc.should_collect());
    }

    #[test]
    fn test_memory_pool() {
        let mut pool: MemoryPool<u32> = MemoryPool::new(10);
        
        let ptr1 = pool.allocate();
        let ptr2 = pool.allocate();
        
        assert_ne!(ptr1, ptr2);
        
        pool.deallocate(ptr1);
        let ptr3 = pool.allocate();
        
        assert_eq!(ptr1, ptr3);
    }
}
