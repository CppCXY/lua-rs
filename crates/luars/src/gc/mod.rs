// Simplified Garbage Collector for Lua VM
//
// Key insight: Objects are already stored in Arena with GcHeader.
// We don't need a separate HashMap to track them!
//
// Design:
// - Arena<GcTable>, Arena<GcFunction>, etc. store all objects
// - GcHeader.marked is used for mark-sweep
// - GC directly iterates over Arena, no extra tracking needed
// - Lua 5.4 style debt mechanism for triggering GC

mod object_pool;

use crate::lua_value::LuaValue;
pub use object_pool::{
    Arena, FunctionId, GcFunction, GcHeader, GcString, GcTable, GcThread, GcUpvalue, ObjectPool,
    StringId, TableId, ThreadId, UpvalueId, UpvalueKey, UpvalueState, UserdataId,
};

// Re-export for compatibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GcObjectType {
    String,
    Table,
    Function,
}

/// Simplified GC state - no HashMap tracking!
pub struct GC {
    // Lua 5.4 GC debt mechanism
    pub(crate) gc_debt: isize,
    pub(crate) total_bytes: usize,

    // GC parameters
    gc_pause: usize, // Pause parameter (default 200 = 200%)
    // gc_step_mul: usize,       // Step multiplier

    // Collection throttling
    check_counter: u32,
    check_interval: u32,

    // Statistics
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
            gc_debt: -(200 * 1024), // Start with 200KB credit
            total_bytes: 0,
            gc_pause: 200,
            check_counter: 0,
            check_interval: 10000,
            stats: GCStats::default(),
        }
    }

    /// Record allocation - just update debt, no HashMap insertion!
    #[inline(always)]
    pub fn register_object(&mut self, _obj_id: u32, obj_type: GcObjectType) {
        let size = match obj_type {
            GcObjectType::String => 64,
            GcObjectType::Table => 256,
            GcObjectType::Function => 128,
        };
        self.total_bytes += size;
        self.gc_debt += size as isize;
    }

    /// Compatibility alias
    #[inline(always)]
    pub fn register_object_tracked(&mut self, obj_id: u32, obj_type: GcObjectType) -> usize {
        self.register_object(obj_id, obj_type);
        obj_id as usize
    }

    /// Record deallocation
    #[inline(always)]
    pub fn record_allocation(&mut self, size: usize) {
        self.total_bytes += size;
        self.gc_debt += size as isize;
    }

    #[inline(always)]
    pub fn record_deallocation(&mut self, size: usize) {
        self.total_bytes = self.total_bytes.saturating_sub(size);
    }

    /// Check if GC should run
    #[inline(always)]
    pub fn should_collect(&self) -> bool {
        self.gc_debt > 0
    }

    #[inline(always)]
    pub fn increment_check_counter(&mut self) {
        self.check_counter += 1;
    }

    #[inline(always)]
    pub fn should_run_collection(&self) -> bool {
        self.check_counter >= self.check_interval
    }

    /// Perform GC step
    pub fn step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        if !self.should_collect() {
            return;
        }

        self.check_counter = 0;
        self.collect(roots, pool);
    }

    /// Main collection - mark and sweep directly on Arena
    pub fn collect(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) -> usize {
        self.stats.collection_count += 1;
        self.stats.major_collections += 1;

        // Phase 1: Clear all marks
        self.clear_marks(pool);

        // Phase 2: Mark from roots
        self.mark_roots(roots, pool);

        // Phase 3: Sweep (free unmarked objects)
        let collected = self.sweep(pool);

        // Update debt
        let alive_estimate =
            pool.tables.len() * 256 + pool.functions.len() * 128 + pool.strings.len() * 64;
        self.gc_debt = -((alive_estimate * self.gc_pause / 100) as isize);

        self.stats.objects_collected += collected;
        collected
    }

    /// Clear all marks in all arenas (skip fixed objects - they stay marked)
    fn clear_marks(&self, pool: &mut ObjectPool) {
        for (_, table) in pool.tables.iter_mut() {
            if !table.header.fixed {
                table.header.marked = false;
            }
        }
        for (_, func) in pool.functions.iter_mut() {
            if !func.header.fixed {
                func.header.marked = false;
            }
        }
        for (_, upval) in pool.upvalues.iter_mut() {
            if !upval.header.fixed {
                upval.header.marked = false;
            }
        }
        for (_, thread) in pool.threads.iter_mut() {
            if !thread.header.fixed {
                thread.header.marked = false;
            }
        }
        for (_, string) in pool.strings.iter_mut() {
            if !string.header.fixed {
                string.header.marked = false;
            }
        }
        // Note: userdata uses Rc internally, no GcHeader
    }

    /// Mark phase - traverse from roots
    /// Uses a worklist algorithm to avoid recursion and handle borrowing correctly
    fn mark_roots(&self, roots: &[LuaValue], pool: &mut ObjectPool) {
        let mut worklist: Vec<LuaValue> = roots.to_vec();

        while let Some(value) = worklist.pop() {
            match value.kind() {
                crate::lua_value::LuaValueKind::Table => {
                    if let Some(id) = value.as_table_id() {
                        if let Some(table) = pool.tables.get_mut(id.0) {
                            if !table.header.marked {
                                table.header.marked = true;
                                // Add table contents to worklist
                                for (k, v) in table.data.iter_all() {
                                    worklist.push(k);
                                    worklist.push(v);
                                }
                                if let Some(mt) = table.data.get_metatable() {
                                    worklist.push(mt);
                                }
                            }
                        }
                    }
                }
                crate::lua_value::LuaValueKind::Function => {
                    if let Some(id) = value.as_function_id() {
                        // First, collect data we need without holding mutable borrow
                        let (should_mark, upvalue_ids, constants) = {
                            if let Some(func) = pool.functions.get(id.0) {
                                if !func.header.marked {
                                    (true, func.upvalues.clone(), func.chunk.constants.clone())
                                } else {
                                    (false, vec![], vec![])
                                }
                            } else {
                                (false, vec![], vec![])
                            }
                        };

                        if should_mark {
                            // Now we can safely mark
                            if let Some(func) = pool.functions.get_mut(id.0) {
                                func.header.marked = true;
                            }

                            // Mark upvalues separately
                            for upval_id in upvalue_ids {
                                if let Some(upval) = pool.upvalues.get_mut(upval_id.0) {
                                    if !upval.header.marked {
                                        upval.header.marked = true;
                                        if let UpvalueState::Closed(v) = &upval.state {
                                            worklist.push(*v);
                                        }
                                    }
                                }
                            }

                            // Add constants to worklist
                            worklist.extend(constants);
                        }
                    }
                }
                crate::lua_value::LuaValueKind::Thread => {
                    if let Some(id) = value.as_thread_id() {
                        // Collect stack values first
                        let stack_values = {
                            if let Some(thread) = pool.threads.get(id.0) {
                                if !thread.header.marked {
                                    Some(thread.data.register_stack.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        if let Some(values) = stack_values {
                            if let Some(thread) = pool.threads.get_mut(id.0) {
                                thread.header.marked = true;
                            }
                            worklist.extend(values);
                        }
                    }
                }
                crate::lua_value::LuaValueKind::Userdata => {
                    // Userdata uses Rc internally, no GC needed
                }
                crate::lua_value::LuaValueKind::String => {
                    // Mark strings (they can be collected if not fixed)
                    if let Some(id) = value.as_string_id() {
                        if let Some(string) = pool.strings.get_mut(id.0) {
                            string.header.marked = true;
                        }
                    }
                }
                _ => {} // Numbers, booleans, nil, CFunction - no marking needed
            }
        }
    }

    /// Sweep phase - free unmarked objects (skip fixed objects)
    fn sweep(&mut self, pool: &mut ObjectPool) -> usize {
        let mut collected = 0;

        // Collect unmarked tables (skip fixed ones)
        let tables_to_free: Vec<u32> = pool
            .tables
            .iter()
            .filter(|(_, t)| !t.header.marked && !t.header.fixed)
            .map(|(id, _)| id)
            .collect();
        for id in tables_to_free {
            pool.tables.free(id);
            collected += 1;
            self.record_deallocation(256);
        }

        // Collect unmarked functions (skip fixed ones)
        let funcs_to_free: Vec<u32> = pool
            .functions
            .iter()
            .filter(|(_, f)| !f.header.marked && !f.header.fixed)
            .map(|(id, _)| id)
            .collect();
        for id in funcs_to_free {
            pool.functions.free(id);
            collected += 1;
            self.record_deallocation(128);
        }

        // Collect unmarked upvalues (skip fixed ones) - using SlotMap
        let upvals_to_free: Vec<UpvalueKey> = pool
            .upvalues
            .iter()
            .filter(|(_, u)| !u.header.marked && !u.header.fixed)
            .map(|(id, _)| id)
            .collect();
        for id in upvals_to_free {
            pool.upvalues.remove(id);
            collected += 1;
        }

        // Collect unmarked threads (skip fixed ones)
        let threads_to_free: Vec<u32> = pool
            .threads
            .iter()
            .filter(|(_, t)| !t.header.marked && !t.header.fixed)
            .map(|(id, _)| id)
            .collect();
        for id in threads_to_free {
            pool.threads.free(id);
            collected += 1;
        }

        // Collect unmarked strings (skip fixed ones)
        // Note: interned strings are usually kept, but this handles non-interned long strings
        let strings_to_free: Vec<u32> = pool
            .strings
            .iter()
            .filter(|(_, s)| !s.header.marked && !s.header.fixed)
            .map(|(id, _)| id)
            .collect();
        for id in strings_to_free {
            pool.strings.free(id);
            collected += 1;
            self.record_deallocation(64);
        }

        // Note: userdata uses Rc internally, no sweep needed

        collected
    }

    /// Write barrier - no-op in simple mark-sweep
    #[inline(always)]
    pub fn barrier_forward(&mut self, _obj_type: GcObjectType, _obj_id: u32) {
        // No-op for simple mark-sweep
    }

    #[inline(always)]
    pub fn barrier_back(&mut self, _value: &LuaValue) {
        // No-op for simple mark-sweep
    }

    #[inline(always)]
    pub fn is_collectable(_value: &LuaValue) -> bool {
        false // Unused
    }

    pub fn unregister_object(&mut self, _obj_id: u32, obj_type: GcObjectType) {
        let size = match obj_type {
            GcObjectType::String => 64,
            GcObjectType::Table => 256,
            GcObjectType::Function => 128,
        };
        self.record_deallocation(size);
    }

    pub fn stats(&self) -> GCStats {
        self.stats.clone()
    }
}

impl Default for GC {
    fn default() -> Self {
        Self::new()
    }
}
