// Generational Garbage Collector for Lua VM
// Implements 2-generation GC with mark-sweep algorithm
// Young generation: frequently collected, most objects die young
// Old generation: rarely collected, long-lived objects

use crate::lua_value::LuaValue;
use crate::{LuaFunction, LuaTable};
use std::collections::{HashMap, HashSet};

/// GC Generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Generation {
    Young,
    Old,
}

/// Object metadata for GC tracking
#[derive(Debug, Clone)]
pub struct GcObject {
    pub obj_id: u32, // The actual object ID (StringId, TableId, etc.)
    pub generation: Generation,
    pub age: u8,
    pub marked: bool,
    pub obj_type: GcObjectType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GcObjectType {
    String,
    Table,
    Function,
}

impl GcObject {
    pub fn new(obj_id: u32, obj_type: GcObjectType) -> Self {
        GcObject {
            obj_id,
            generation: Generation::Young,
            age: 0,
            marked: false,
            obj_type,
        }
    }

    pub fn promote(&mut self) {
        self.generation = Generation::Old;
        self.age = 0;
    }

    pub fn mark(&mut self) {
        self.marked = true;
    }

    pub fn unmark(&mut self) {
        self.marked = false;
    }

    pub fn increment_age(&mut self) -> bool {
        if self.generation == Generation::Young {
            self.age += 1;
            self.age >= 3 // Promote after 3 minor GCs
        } else {
            false
        }
    }
}

/// Garbage collector state
pub struct GC {
    // Object tracking for generational GC - key is (type, id)
    objects: HashMap<(GcObjectType, u32), GcObject>,
    next_gc_id: usize, // Internal GC tracking ID

    // GC triggers
    allocations_since_minor_gc: usize,
    minor_gc_count: usize,
    minor_gc_threshold: usize,
    major_gc_threshold: usize,

    // Total bytes allocated
    bytes_allocated: usize,

    // GC threshold (trigger collection when exceeded)
    threshold: usize,

    // Statistics
    collection_count: usize,
    stats: GCStats,
}

#[derive(Debug, Clone, Default)]
pub struct GCStats {
    pub bytes_allocated: usize,
    pub threshold: usize,
    pub collection_count: usize,
    pub minor_collections: usize,
    pub major_collections: usize,
    pub objects_collected: usize,
    pub young_gen_size: usize,
    pub old_gen_size: usize,
    pub promoted_objects: usize,
}

impl GC {
    pub fn new() -> Self {
        GC {
            objects: HashMap::new(),
            next_gc_id: 1,
            allocations_since_minor_gc: 0,
            minor_gc_count: 0,
            minor_gc_threshold: 10000,
            major_gc_threshold: 50,
            bytes_allocated: 0,
            threshold: 8 * 1024 * 1024, // 8MB initial threshold
            collection_count: 0,
            stats: GCStats::default(),
        }
    }

    /// Register a new object for GC tracking
    pub fn register_object(&mut self, obj_id: u32, obj_type: GcObjectType) -> usize {
        let gc_id = self.next_gc_id;
        self.next_gc_id += 1;

        let obj = GcObject::new(obj_id, obj_type);
        self.objects.insert((obj_type, obj_id), obj);
        self.allocations_since_minor_gc += 1;

        let size = match obj_type {
            GcObjectType::String => 64,
            GcObjectType::Table => 256,
            GcObjectType::Function => 128,
        };
        self.record_allocation(size);

        gc_id
    }

    /// Unregister an object (when explicitly deleted)
    pub fn unregister_object(&mut self, obj_id: u32, obj_type: GcObjectType) {
        if let Some(obj) = self.objects.remove(&(obj_type, obj_id)) {
            let size = match obj.obj_type {
                GcObjectType::String => 64,
                GcObjectType::Table => 256,
                GcObjectType::Function => 128,
            };
            self.record_deallocation(size);
        }
    }

    /// Check if GC should run
    pub fn should_collect(&self) -> bool {
        self.bytes_allocated > self.threshold || self.should_collect_young()
    }

    pub fn should_collect_young(&self) -> bool {
        self.allocations_since_minor_gc >= self.minor_gc_threshold
    }

    pub fn should_collect_old(&self) -> bool {
        self.minor_gc_count >= self.major_gc_threshold
    }

    /// Perform garbage collection (chooses minor or major)
    /// Takes root set (stack, globals, etc.) and marks reachable objects
    pub fn collect(
        &mut self,
        roots: &[LuaValue],
        object_pool: &mut crate::object_pool::ObjectPool,
    ) -> usize {
        if self.should_collect_old() {
            self.major_collect(roots, object_pool)
        } else {
            self.minor_collect(roots, object_pool)
        }
    }

    /// Minor GC - collect young generation only
    fn minor_collect(
        &mut self,
        roots: &[LuaValue],
        object_pool: &mut crate::object_pool::ObjectPool,
    ) -> usize {
        self.collection_count += 1;
        self.stats.minor_collections += 1;

        let reachable = self.mark(roots, object_pool);

        let mut collected = 0;
        let mut promoted = 0;
        let mut survivors = Vec::new();

        // Get all young generation objects
        let young_objs: Vec<(GcObjectType, u32)> = self
            .objects
            .iter()
            .filter(|(_, obj)| obj.generation == Generation::Young)
            .map(|(key, _)| *key)
            .collect();

        for key @ (obj_type, obj_id) in young_objs {
            if let Some(mut obj) = self.objects.remove(&key) {
                if reachable.contains(&key) {
                    // Object survived
                    obj.unmark();

                    if obj.increment_age() {
                        obj.promote();
                        promoted += 1;
                    }

                    survivors.push((key, obj));
                } else {
                    // Collect garbage - remove from object pool!
                    collected += 1;
                    match obj_type {
                        GcObjectType::String => {
                            let string_id = crate::object_pool::StringId(obj_id);
                            object_pool.remove_string(string_id);
                        }
                        GcObjectType::Table => {
                            let table_id = crate::object_pool::TableId(obj_id);
                            object_pool.remove_table(table_id);
                        }
                        GcObjectType::Function => {
                            let func_id = crate::object_pool::FunctionId(obj_id);
                            object_pool.remove_function(func_id);
                        }
                    }

                    let size = match obj_type {
                        GcObjectType::String => 64,
                        GcObjectType::Table => 256,
                        GcObjectType::Function => 128,
                    };
                    self.record_deallocation(size);
                }
            }
        }

        // Re-insert survivors
        for (key, obj) in survivors {
            self.objects.insert(key, obj);
        }

        self.stats.objects_collected += collected;
        self.stats.promoted_objects += promoted;
        self.update_generation_sizes();

        self.allocations_since_minor_gc = 0;
        self.minor_gc_count += 1;

        collected
    }

    /// Major GC - collect all generations
    fn major_collect(
        &mut self,
        roots: &[LuaValue],
        object_pool: &mut crate::object_pool::ObjectPool,
    ) -> usize {
        self.collection_count += 1;
        self.stats.major_collections += 1;

        let reachable = self.mark(roots, object_pool);

        let mut collected = 0;
        let keys: Vec<(GcObjectType, u32)> = self.objects.keys().copied().collect();

        for key @ (obj_type, obj_id) in keys {
            if let Some(obj) = self.objects.get(&key) {
                if !reachable.contains(&key) {
                    self.objects.remove(&key);
                    collected += 1;

                    // Remove from object pool!
                    match obj_type {
                        GcObjectType::String => {
                            let string_id = crate::object_pool::StringId(obj_id);
                            object_pool.remove_string(string_id);
                        }
                        GcObjectType::Table => {
                            let table_id = crate::object_pool::TableId(obj_id);
                            object_pool.remove_table(table_id);
                        }
                        GcObjectType::Function => {
                            let func_id = crate::object_pool::FunctionId(obj_id);
                            object_pool.remove_function(func_id);
                        }
                    }

                    let size = match obj_type {
                        GcObjectType::String => 64,
                        GcObjectType::Table => 256,
                        GcObjectType::Function => 128,
                    };
                    self.record_deallocation(size);
                }
            }
        }

        // Unmark all survivors
        for obj in self.objects.values_mut() {
            obj.unmark();
        }

        self.stats.objects_collected += collected;
        self.update_generation_sizes();

        self.minor_gc_count = 0;
        self.allocations_since_minor_gc = 0;
        self.adjust_threshold();

        collected
    }

    /// Mark phase: traverse object graph from roots
    fn mark(
        &mut self,
        roots: &[LuaValue],
        object_pool: &crate::object_pool::ObjectPool,
    ) -> HashSet<(GcObjectType, u32)> {
        let mut marked = HashSet::new();
        let mut worklist: Vec<LuaValue> = roots.to_vec();

        while let Some(value) = worklist.pop() {
            // Get object key (type, id)
            let key = match value.kind() {
                crate::lua_value::LuaValueKind::String => {
                    value.as_string_id().map(|id| (GcObjectType::String, id.0))
                }
                crate::lua_value::LuaValueKind::Table => {
                    value.as_table_id().map(|id| (GcObjectType::Table, id.0))
                }
                crate::lua_value::LuaValueKind::Function => value
                    .as_function_id()
                    .map(|id| (GcObjectType::Function, id.0)),
                _ => None,
            };

            if let Some(key) = key {
                if marked.contains(&key) {
                    continue;
                }

                marked.insert(key);

                // Mark the object
                if let Some(obj) = self.objects.get_mut(&key) {
                    obj.mark();
                }

                // Mark children
                match key.0 {
                    GcObjectType::Table => {
                        if let Some(table_ref) =
                            object_pool.get_table(crate::object_pool::TableId(key.1))
                        {
                            self.mark_table(&table_ref.borrow(), &mut worklist);
                        }
                    }
                    GcObjectType::Function => {
                        if let Some(func_ref) =
                            object_pool.get_function(crate::object_pool::FunctionId(key.1))
                        {
                            self.mark_function(&func_ref.borrow(), &mut worklist);
                        }
                    }
                    _ => {}
                }
            }
        }

        marked
    }

    /// Mark table contents
    fn mark_table(&self, table: &LuaTable, worklist: &mut Vec<LuaValue>) {
        // Mark both keys and values
        for (key, val) in table.iter_all() {
            worklist.push(key);
            worklist.push(val);
        }
    }

    /// Mark function upvalues
    fn mark_function(&self, func: &LuaFunction, worklist: &mut Vec<LuaValue>) {
        for upval in &func.upvalues {
            // Only mark closed upvalues (open ones are on the stack already)
            if let Some(val) = upval.get_closed_value() {
                worklist.push(val);
            }
        }
    }

    /// Update generation size statistics
    fn update_generation_sizes(&mut self) {
        let (young, old) = self
            .objects
            .values()
            .fold((0, 0), |(y, o), obj| match obj.generation {
                Generation::Young => (y + 1, o),
                Generation::Old => (y, o + 1),
            });

        self.stats.young_gen_size = young;
        self.stats.old_gen_size = old;
    }

    /// Adjust GC threshold based on current usage
    fn adjust_threshold(&mut self) {
        // Grow threshold based on current allocation
        self.threshold = (self.bytes_allocated * 2).max(1024 * 1024);
    }

    /// Record allocation
    pub fn record_allocation(&mut self, size: usize) {
        self.bytes_allocated += size;
        self.stats.bytes_allocated = self.bytes_allocated;
    }

    /// Record deallocation
    pub fn record_deallocation(&mut self, size: usize) {
        self.bytes_allocated = self.bytes_allocated.saturating_sub(size);
        self.stats.bytes_allocated = self.bytes_allocated;
    }

    /// Get statistics
    pub fn stats(&self) -> GCStats {
        self.stats.clone()
    }

    /// Tune GC thresholds based on current state
    pub fn tune_thresholds(&mut self) {
        let total = self.stats.young_gen_size + self.stats.old_gen_size;
        self.minor_gc_threshold = (total / 2).max(50).min(500);

        if self.stats.old_gen_size > 1000 {
            self.major_gc_threshold = 5;
        } else {
            self.major_gc_threshold = 10;
        }
    }
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

        // GC threshold is 8MB by default
        gc.record_allocation(9 * 1024 * 1024); // 9MB > 8MB threshold
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
