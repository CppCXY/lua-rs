// Garbage Collector for Lua VM
//
// Design based on Lua 5.4 but adapted for Rust:
// - GcId: Unified object identifier (type tag + pool index)
// - allgc: Vec<GcId> tracking all collectable objects (like Lua's allgc list)
// - Tri-color marking: white (unmarked), gray (marked, refs not scanned), black (fully scanned)
// - Generational collection: objects have ages (NEW, SURVIVAL, OLD)
// - Sweep only traverses allgc, not entire pools
//
// Key difference from Lua C: We use Vec<GcId> instead of linked list
// because Rust ownership makes linked lists impractical

mod object_pool;

use crate::lua_value::LuaValue;
pub use object_pool::{
    Arena, BoxPool, FunctionId, GcFunction, GcHeader, GcString, GcTable, GcThread, GcUpvalue,
    ObjectPool, Pool, StringId, TableId, ThreadId, UpvalueId, UpvalueState, UserdataId,
};

// ============ GcId: Unified Object Identifier ============

/// Object type tags (3 bits, supports up to 8 types)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcType {
    String = 0,
    Table = 1,
    Function = 2,
    Upvalue = 3,
    Thread = 4,
    Userdata = 5,
}

/// Unified GC object identifier
/// Layout: [type: 3 bits][index: 29 bits]
/// Supports up to 536 million objects per type
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct GcId(u32);

impl GcId {
    const TYPE_BITS: u32 = 3;
    const TYPE_MASK: u32 = (1 << Self::TYPE_BITS) - 1;
    const INDEX_SHIFT: u32 = Self::TYPE_BITS;

    #[inline(always)]
    pub fn new(gc_type: GcType, index: u32) -> Self {
        debug_assert!(index < (1 << 29), "Index overflow");
        GcId((index << Self::INDEX_SHIFT) | (gc_type as u32))
    }

    #[inline(always)]
    pub fn gc_type(self) -> GcType {
        unsafe { std::mem::transmute((self.0 & Self::TYPE_MASK) as u8) }
    }

    #[inline(always)]
    pub fn index(self) -> u32 {
        self.0 >> Self::INDEX_SHIFT
    }

    #[inline(always)]
    pub fn from_string(id: StringId) -> Self {
        Self::new(GcType::String, id.0)
    }

    #[inline(always)]
    pub fn from_table(id: TableId) -> Self {
        Self::new(GcType::Table, id.0)
    }

    #[inline(always)]
    pub fn from_function(id: FunctionId) -> Self {
        Self::new(GcType::Function, id.0)
    }

    #[inline(always)]
    pub fn from_upvalue(id: UpvalueId) -> Self {
        Self::new(GcType::Upvalue, id.0)
    }

    #[inline(always)]
    pub fn from_thread(id: ThreadId) -> Self {
        Self::new(GcType::Thread, id.0)
    }

    #[inline(always)]
    pub fn as_string_id(self) -> Option<StringId> {
        if self.gc_type() == GcType::String {
            Some(StringId(self.index()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_table_id(self) -> Option<TableId> {
        if self.gc_type() == GcType::Table {
            Some(TableId(self.index()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_function_id(self) -> Option<FunctionId> {
        if self.gc_type() == GcType::Function {
            Some(FunctionId(self.index()))
        } else {
            None
        }
    }
}

// Re-export for compatibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GcObjectType {
    String,
    Table,
    Function,
}

/// Object age for generational GC (like Lua 5.4)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GcAge {
    New = 0,      // Created in current cycle
    Survival = 1, // Survived one collection
    Old0 = 2,     // Marked old by barrier in this cycle
    Old1 = 3,     // First full cycle as old
    Old = 4,      // Really old (rarely visited)
    Touched1 = 5, // Old object touched this cycle
    Touched2 = 6, // Old object touched in previous cycle
}

/// GC color for tri-color marking
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcColor {
    White0 = 0, // Unmarked (current white)
    White1 = 1, // Unmarked (other white, for flip)
    Gray = 2,   // Marked, refs not yet scanned
    Black = 3,  // Fully marked
}

/// GC state machine phases
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcState {
    Pause = 0,     // Between cycles
    Propagate = 1, // Marking phase
    Atomic = 2,    // Atomic finish of marking
    Sweep = 3,     // Sweeping dead objects
}

/// Main GC structure
pub struct GC {
    // Object tracking (like Lua's allgc list but using Vec)
    pub(crate) allgc: Vec<GcId>,
    
    // Gray list for incremental marking
    gray: Vec<GcId>,
    grayagain: Vec<GcId>,
    
    // Lua 5.4 GC debt mechanism
    pub(crate) gc_debt: isize,
    pub(crate) total_bytes: usize,

    // GC state
    state: GcState,
    current_white: u8, // 0 or 1, flips each cycle

    // GC parameters
    gc_pause: usize,     // Pause parameter (default 200 = 200%)
    gc_stepmul: usize,   // Step multiplier (default 100)
    
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
            allgc: Vec::with_capacity(1024),
            gray: Vec::with_capacity(256),
            grayagain: Vec::with_capacity(64),
            gc_debt: -(200 * 1024), // Start with 200KB credit
            total_bytes: 0,
            state: GcState::Pause,
            current_white: 0,
            gc_pause: 200,
            gc_stepmul: 100,
            check_counter: 0,
            check_interval: 1, // Run GC every check_gc_slow call
            stats: GCStats::default(),
        }
    }

    /// Register a new object for GC tracking
    /// This is called when an object is allocated
    #[inline(always)]
    pub fn track_object(&mut self, gc_id: GcId, size: usize) {
        self.allgc.push(gc_id);
        self.total_bytes += size;
        self.gc_debt += size as isize;
    }

    /// Record allocation - compatibility with old API
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
        // Run GC if debt is high OR if allgc has grown too large
        // This prevents allgc from growing unbounded even if gc_debt is reset
        let should_run = self.should_collect() || self.allgc.len() > 50000;
        if !should_run {
            return;
        }

        self.check_counter = 0;
        self.collect(roots, pool);
    }

    /// Main collection - mark and sweep using allgc list
    pub fn collect(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) -> usize {
        self.stats.collection_count += 1;
        self.stats.major_collections += 1;

        // Phase 1: Clear all marks (only for tracked objects)
        self.clear_marks(pool);

        // Phase 2: Mark from roots
        self.mark_roots(roots, pool);

        // Phase 3: Sweep (only traverse allgc, not entire pools!)
        let collected = self.sweep(pool);

        // Update debt based on survivors
        let alive_bytes = self.allgc.len() * 128; // Average size estimate
        self.gc_debt = -((alive_bytes * self.gc_pause / 100) as isize);

        self.stats.objects_collected += collected;
        collected
    }

    /// Clear marks only for tracked objects (much faster than iterating all pools)
    fn clear_marks(&self, pool: &mut ObjectPool) {
        for &gc_id in &self.allgc {
            match gc_id.gc_type() {
                GcType::Table => {
                    if let Some(t) = pool.tables.get_mut(gc_id.index()) {
                        if !t.header.fixed {
                            t.header.marked = false;
                        }
                    }
                }
                GcType::Function => {
                    if let Some(f) = pool.functions.get_mut(gc_id.index()) {
                        if !f.header.fixed {
                            f.header.marked = false;
                        }
                    }
                }
                GcType::Upvalue => {
                    if let Some(u) = pool.upvalues.get_mut(gc_id.index()) {
                        if !u.header.fixed {
                            u.header.marked = false;
                        }
                    }
                }
                GcType::Thread => {
                    if let Some(t) = pool.threads.get_mut(gc_id.index()) {
                        if !t.header.fixed {
                            t.header.marked = false;
                        }
                    }
                }
                GcType::String => {
                    if let Some(s) = pool.strings.get_mut(gc_id.index()) {
                        if !s.header.fixed {
                            s.header.marked = false;
                        }
                    }
                }
                GcType::Userdata => {} // Rc handles this
            }
        }
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

    /// Sweep phase - only traverse allgc list, not entire pools!
    /// This is the key optimization: we only look at objects we're tracking
    fn sweep(&mut self, pool: &mut ObjectPool) -> usize {
        let mut collected = 0;
        let mut write_idx = 0;

        // Sweep using allgc - only traverse tracked objects
        for read_idx in 0..self.allgc.len() {
            let gc_id = self.allgc[read_idx];
            let (is_marked, is_fixed) = self.get_object_state(gc_id, pool);

            if is_marked || is_fixed {
                // Object survives - keep it in allgc
                self.allgc[write_idx] = gc_id;
                write_idx += 1;
            } else {
                // Object is dead - free it
                self.free_object(gc_id, pool);
                collected += 1;
            }
        }

        // Truncate allgc to remove dead entries
        self.allgc.truncate(write_idx);

        collected
    }

    /// Get marked and fixed state for an object
    #[inline]
    fn get_object_state(&self, gc_id: GcId, pool: &ObjectPool) -> (bool, bool) {
        match gc_id.gc_type() {
            GcType::Table => {
                if let Some(t) = pool.tables.get(gc_id.index()) {
                    (t.header.marked, t.header.fixed)
                } else {
                    (false, false)
                }
            }
            GcType::Function => {
                if let Some(f) = pool.functions.get(gc_id.index()) {
                    (f.header.marked, f.header.fixed)
                } else {
                    (false, false)
                }
            }
            GcType::Upvalue => {
                if let Some(u) = pool.upvalues.get(gc_id.index()) {
                    (u.header.marked, u.header.fixed)
                } else {
                    (false, false)
                }
            }
            GcType::Thread => {
                if let Some(t) = pool.threads.get(gc_id.index()) {
                    (t.header.marked, t.header.fixed)
                } else {
                    (false, false)
                }
            }
            GcType::String => {
                if let Some(s) = pool.strings.get(gc_id.index()) {
                    (s.header.marked, s.header.fixed)
                } else {
                    (false, false)
                }
            }
            GcType::Userdata => (true, true), // Userdata uses Rc, always "alive"
        }
    }

    /// Free an object from its pool
    #[inline]
    fn free_object(&mut self, gc_id: GcId, pool: &mut ObjectPool) {
        match gc_id.gc_type() {
            GcType::Table => {
                pool.tables.free(gc_id.index());
                self.record_deallocation(256);
            }
            GcType::Function => {
                pool.functions.free(gc_id.index());
                self.record_deallocation(128);
            }
            GcType::Upvalue => {
                pool.upvalues.free(gc_id.index());
                self.record_deallocation(64);
            }
            GcType::Thread => {
                pool.threads.free(gc_id.index());
                self.record_deallocation(512);
            }
            GcType::String => {
                pool.strings.free(gc_id.index());
                self.record_deallocation(64);
            }
            GcType::Userdata => {} // Rc handles this
        }
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
