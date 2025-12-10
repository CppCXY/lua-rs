// Garbage Collector for Lua VM
//
// Design based on Lua 5.4 with full Generational GC support:
// - GcId: Unified object identifier (type tag + pool index)
// - Dual-mode: Incremental (KGC_INC) or Generational (KGC_GEN)
// - Tri-color marking: white, gray, black
// - Generational: objects have ages (NEW, SURVIVAL, OLD0, OLD1, OLD, TOUCHED1, TOUCHED2)
// - Minor collection: Only collect young generation
// - Major collection: Full collection when memory grows too much
//
// Key difference from Lua C: We use Vec<GcId> instead of linked list
// and iterate pools directly for sweeping (allocation is O(1) via free list)

mod gc_id;
mod gc_object;
mod object_pool;

use crate::lua_value::{LuaValue, LuaValueKind};
pub use gc_id::*;
pub use gc_object::*;
pub use object_pool::*;

/// GC mode: Incremental or Generational
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcKind {
    Incremental = 0,  // Traditional incremental mark-sweep
    Generational = 1, // Generational with minor/major collections
}

/// Object age for generational GC (like Lua 5.4)
/// Age transitions:
/// - NEW → SURVIVAL (after surviving a minor collection)
/// - SURVIVAL → OLD1 (after surviving another minor)
/// - OLD0 → OLD1 (barrier promoted objects)
/// - OLD1 → OLD (after another collection)
/// - TOUCHED1 → TOUCHED2 → OLD (old objects that got a back barrier)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GcAge {
    New = 0,      // Created in current cycle
    Survival = 1, // Survived one minor collection
    Old0 = 2,     // Marked old by forward barrier in this cycle
    Old1 = 3,     // First full cycle as old
    Old = 4,      // Really old (not visited in minor GC)
    Touched1 = 5, // Old object touched this cycle (back barrier)
    Touched2 = 6, // Old object touched in previous cycle
}

impl GcAge {
    /// Get the next age after a collection cycle
    #[inline]
    pub fn next_age(self) -> GcAge {
        match self {
            GcAge::New => GcAge::Survival,
            GcAge::Survival => GcAge::Old1,
            GcAge::Old0 => GcAge::Old1,
            GcAge::Old1 => GcAge::Old,
            GcAge::Old => GcAge::Old,
            GcAge::Touched1 => GcAge::Touched1, // handled specially
            GcAge::Touched2 => GcAge::Touched2, // handled specially
        }
    }

    /// Check if this age is considered "old"
    #[inline]
    pub fn is_old(self) -> bool {
        self as u8 >= GcAge::Old0 as u8
    }
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
/// Supports both incremental and generational modes (like Lua 5.4)
pub struct GC {
    // Gray lists for marking
    gray: Vec<GcId>,
    grayagain: Vec<GcId>,

    // Lua 5.4 GC debt mechanism
    pub(crate) gc_debt: isize,
    pub(crate) total_bytes: usize,

    // GC state machine
    state: GcState,
    current_white: u8, // 0 or 1, flips each cycle

    // GC mode (incremental or generational)
    gc_kind: GcKind,

    // Incremental sweep state
    sweep_index: usize,    // Current position in sweep phase
    propagate_work: usize, // Work done in propagate phase

    // GC parameters (like Lua's gcparam)
    gc_pause: usize,      // Pause parameter (default 200 = 200%)
    gen_minor_mul: usize, // Minor collection multiplier (default 25 = 25%)
    gen_major_mul: usize, // Major collection threshold (default 100 = 100%)

    // Generational mode state
    last_atomic: usize, // Objects traversed in last atomic (0 = good collection)
    pub(crate) gc_estimate: usize, // Estimate of memory in use after major collection

    // Generation boundaries (indices into allgc-style tracking)
    // In our design, we track these via object ages in headers
    // These are used for the minor collection optimization
    young_list: Vec<GcId>,   // NEW and SURVIVAL objects
    touched_list: Vec<GcId>, // TOUCHED1 and TOUCHED2 old objects

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
            gray: Vec::with_capacity(256),
            grayagain: Vec::with_capacity(64),
            // Start with negative debt like Lua
            // gc_debt < 0 means "credit" before next collection
            gc_debt: -(8 * 1024), // 8KB credit before first GC
            total_bytes: 0,
            state: GcState::Pause,
            current_white: 0,
            gc_kind: GcKind::Generational, // Use generational mode like Lua 5.4
            sweep_index: 0,
            propagate_work: 0,
            gc_pause: 200,
            gen_minor_mul: 20, // Minor GC when memory grows 20%
            gen_major_mul: 50, // Major GC when memory grows 50% (降低以更频繁触发major GC清除weak tables)
            last_atomic: 0,
            gc_estimate: 0,
            young_list: Vec::with_capacity(1024),
            touched_list: Vec::with_capacity(256),
            check_counter: 0,
            check_interval: 1,
            stats: GCStats::default(),
        }
    }

    /// Create GC in incremental mode (for compatibility/testing)
    pub fn new_incremental() -> Self {
        let mut gc = Self::new();
        gc.gc_kind = GcKind::Incremental;
        gc
    }

    /// Get current GC mode
    #[inline]
    pub fn gc_kind(&self) -> GcKind {
        self.gc_kind
    }

    /// Set GC mode
    pub fn set_gc_kind(&mut self, kind: GcKind) {
        self.gc_kind = kind;
    }

    /// Register a new object for GC tracking
    /// In generational mode, new objects are added to young_list
    #[inline(always)]
    pub fn track_object(&mut self, gc_id: GcId, size: usize) {
        self.total_bytes += size;
        self.gc_debt += size as isize;

        // In generational mode, track new objects in young_list
        if self.gc_kind == GcKind::Generational {
            self.young_list.push(gc_id);
        }
    }

    /// Record allocation - compatibility with old API
    #[inline(always)]
    pub fn register_object(&mut self, _obj_id: u32, obj_type: GcObjectType) {
        let size = match obj_type {
            GcObjectType::String => 64,
            GcObjectType::Table => 256,
            GcObjectType::Function => 128,
            GcObjectType::Upvalue => 64,
            GcObjectType::Thread => 512,
            GcObjectType::Userdata => 32,
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

    /// Perform GC step - like Lua's luaC_step
    /// Dispatches to incremental or generational mode based on gc_kind
    pub fn step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // Like Lua: run GC when debt > 0
        if self.gc_debt <= 0 {
            return;
        }

        match self.gc_kind {
            GcKind::Generational => self.gen_step(roots, pool),
            GcKind::Incremental => self.inc_step(roots, pool),
        }
    }

    /// Generational GC step - like Lua's genstep
    fn gen_step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // First time entering generational mode: initialize gc_estimate
        // Like Lua's entergen(), we need to do an initial full collection
        if self.gc_estimate == 0 && self.last_atomic == 0 {
            // First collection: do a full gen to initialize gc_estimate
            self.full_gen(roots, pool);
            self.set_minor_debt();
            return;
        }

        if self.last_atomic != 0 {
            // Last collection was bad, do a full step
            self.step_gen_full(roots, pool);
        } else {
            // Check if we need a major collection
            let major_base = self.gc_estimate;
            let major_inc = (major_base / 100) * self.gen_major_mul;

            if self.gc_debt > 0 && self.total_bytes > major_base + major_inc {
                // Memory grew too much, do a major collection
                let num_objs = self.full_gen(roots, pool);

                if self.total_bytes < major_base + (major_inc / 2) {
                    // Good collection - collected at least half of growth
                    self.last_atomic = 0;
                } else {
                    // Bad collection
                    self.last_atomic = num_objs;
                    self.set_pause();
                }
            } else {
                // Regular case: do a minor collection
                self.young_collection(roots, pool);
                self.set_minor_debt();
            }
        }
    }

    /// Incremental GC step - complete one full cycle
    fn inc_step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // Pause -> Start cycle and mark roots
        if self.state == GcState::Pause {
            self.start_cycle(roots, pool);
            self.state = GcState::Propagate;
        }

        // Propagate -> Mark all gray objects
        if self.state == GcState::Propagate {
            while let Some(gc_id) = self.gray.pop() {
                self.mark_one(gc_id, pool);
            }
            self.state = GcState::Atomic;
        }

        // Atomic -> Process grayagain and clear weak tables
        if self.state == GcState::Atomic {
            while let Some(gc_id) = self.grayagain.pop() {
                self.mark_one(gc_id, pool);
            }
            self.clear_weak_tables(pool);
            self.sweep_index = 0;
            self.state = GcState::Sweep;
        }

        // Sweep -> Clean up dead objects
        if self.state == GcState::Sweep {
            self.sweep_step(pool, usize::MAX);
        }
    }

    /// Start a new GC cycle - mark roots and build gray list
    fn start_cycle(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.stats.collection_count += 1;
        self.gray.clear();
        self.grayagain.clear();
        self.propagate_work = 0;

        // Make all objects white by iterating pools directly
        for (_id, table) in pool.tables.iter_mut() {
            if !table.header.is_fixed() {
                table.header.make_white(self.current_white);
            }
        }
        for (_id, func) in pool.functions.iter_mut() {
            if !func.header.is_fixed() {
                func.header.make_white(self.current_white);
            }
        }
        for (_id, upval) in pool.upvalues.iter_mut() {
            if !upval.header.is_fixed() {
                upval.header.make_white(self.current_white);
            }
        }
        for (_id, thread) in pool.threads.iter_mut() {
            if !thread.header.is_fixed() {
                thread.header.make_white(self.current_white);
            }
        }
        for (_id, string) in pool.strings.iter_mut() {
            if !string.header.is_fixed() {
                string.header.make_white(self.current_white);
            }
        }

        // Mark roots and add to gray list
        for value in roots {
            self.mark_value(value, pool);
        }
    }

    /// Make an object white (for start of cycle)
    #[inline]
    fn make_white(&self, gc_id: GcId, pool: &mut ObjectPool) {
        match gc_id.gc_type() {
            GcObjectType::Table => {
                if let Some(t) = pool.tables.get_mut(gc_id.index()) {
                    if !t.header.is_fixed() {
                        t.header.make_white(self.current_white);
                    }
                }
            }
            GcObjectType::Function => {
                if let Some(f) = pool.functions.get_mut(gc_id.index()) {
                    if !f.header.is_fixed() {
                        f.header.make_white(self.current_white);
                    }
                }
            }
            GcObjectType::Upvalue => {
                if let Some(u) = pool.upvalues.get_mut(gc_id.index()) {
                    if !u.header.is_fixed() {
                        u.header.make_white(self.current_white);
                    }
                }
            }
            GcObjectType::Thread => {
                if let Some(t) = pool.threads.get_mut(gc_id.index()) {
                    if !t.header.is_fixed() {
                        t.header.make_white(self.current_white);
                    }
                }
            }
            GcObjectType::String => {
                if let Some(s) = pool.strings.get_mut(gc_id.index()) {
                    if !s.header.is_fixed() {
                        s.header.make_white(self.current_white);
                    }
                }
            }
            GcObjectType::Userdata => {}
        }
    }

    /// Mark a value and add to gray list if needed
    fn mark_value(&mut self, value: &LuaValue, pool: &mut ObjectPool) {
        match value.kind() {
            LuaValueKind::Table => {
                if let Some(id) = value.as_table_id() {
                    if let Some(t) = pool.tables.get_mut(id.0) {
                        if t.header.is_white() {
                            t.header.make_gray();
                            self.gray.push(GcId::TableId(id));
                        }
                    }
                }
            }
            LuaValueKind::Function => {
                if let Some(id) = value.as_function_id() {
                    if let Some(f) = pool.functions.get_mut(id.0) {
                        if f.header.is_white() {
                            f.header.make_gray();
                            self.gray.push(GcId::FunctionId(id));
                        }
                    }
                }
            }
            LuaValueKind::Thread => {
                if let Some(id) = value.as_thread_id() {
                    if let Some(t) = pool.threads.get_mut(id.0) {
                        if t.header.is_white() {
                            t.header.make_gray();
                            self.gray.push(GcId::ThreadId(id));
                        }
                    }
                }
            }
            LuaValueKind::String => {
                if let Some(id) = value.as_string_id() {
                    if let Some(s) = pool.strings.get_mut(id.0) {
                        // Strings are leaves - mark black directly
                        s.header.make_black();
                    }
                }
            }
            _ => {}
        }
    }

    /// Do one step of propagation - process some gray objects
    #[allow(unused)]
    fn propagate_step(&mut self, pool: &mut ObjectPool, max_work: usize) -> usize {
        let mut work = 0;

        while work < max_work {
            if let Some(gc_id) = self.gray.pop() {
                work += self.mark_one(gc_id, pool);
            } else {
                break;
            }
        }

        work
    }

    /// Mark one gray object and its references
    fn mark_one(&mut self, gc_id: GcId, pool: &mut ObjectPool) -> usize {
        let mut work = 1;

        match gc_id.gc_type() {
            GcObjectType::Table => {
                // First pass: collect all needed info without mutating
                let (should_mark, entries, mt_value, weak_mode) = {
                    if let Some(table) = pool.tables.get(gc_id.index()) {
                        if table.header.is_gray() {
                            let entries = table.data.iter_all();
                            let mt = table.data.get_metatable();

                            // Check weak mode from metatable
                            let weak = if let Some(mt_id) = mt.and_then(|v| v.as_table_id()) {
                                if let Some(mt_table) = pool.tables.get(mt_id.0) {
                                    self.get_weak_mode(mt_table, pool)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            (true, entries, mt, weak)
                        } else {
                            (false, vec![], None, None)
                        }
                    } else {
                        (false, vec![], None, None)
                    }
                };

                if should_mark {
                    // Now mark the table as black
                    if let Some(table) = pool.tables.get_mut(gc_id.index()) {
                        table.header.make_black();
                        work += table.data.len();
                    }

                    let (weak_keys, weak_values) = weak_mode.unwrap_or((false, false));

                    // Mark references (skip weak references)
                    for (k, v) in entries {
                        if !weak_keys {
                            self.mark_value(&k, pool);
                        }
                        if !weak_values {
                            self.mark_value(&v, pool);
                        }
                    }
                    if let Some(mt) = mt_value {
                        self.mark_value(&mt, pool);
                    }
                }
            }
            GcObjectType::Function => {
                if let Some(func) = pool.functions.get(gc_id.index()) {
                    let upvalue_ids = func.upvalues.clone();
                    // C closures don't have constants
                    let constants = func
                        .chunk()
                        .map(|c| c.constants.clone())
                        .unwrap_or_default();

                    if let Some(f) = pool.functions.get_mut(gc_id.index()) {
                        if f.header.is_gray() {
                            f.header.make_black();
                            work += upvalue_ids.len() + constants.len();
                        }
                    }

                    // Mark upvalues
                    for upval_id in upvalue_ids {
                        if let Some(upval) = pool.upvalues.get_mut(upval_id.0) {
                            if upval.header.is_white() {
                                upval.header.make_gray();
                                self.gray.push(GcId::UpvalueId(upval_id));
                            }
                        }
                    }

                    // Mark constants
                    for c in constants {
                        self.mark_value(&c, pool);
                    }
                }
            }
            GcObjectType::Upvalue => {
                // Get closed value first, then release borrow before marking
                let closed_value = if let Some(upval) = pool.upvalues.get_mut(gc_id.index()) {
                    if upval.header.is_gray() {
                        upval.header.make_black();
                        if !upval.is_open {
                            Some(upval.closed_value)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(v) = closed_value {
                    self.mark_value(&v, pool);
                }
            }
            GcObjectType::Thread => {
                if let Some(thread) = pool.threads.get(gc_id.index()) {
                    let stack = thread.data.register_stack.clone();

                    if let Some(t) = pool.threads.get_mut(gc_id.index()) {
                        if t.header.is_gray() {
                            t.header.make_black();
                            work += stack.len();
                        }
                    }

                    for v in stack {
                        self.mark_value(&v, pool);
                    }
                }
            }
            GcObjectType::String => {
                // Strings are leaves, just make black
                if let Some(s) = pool.strings.get_mut(gc_id.index()) {
                    s.header.make_black();
                }
            }
            GcObjectType::Userdata => {}
        }

        work
    }

    /// Do one step of sweeping - sweep all pools in one step
    /// This is acceptable because sweep is much faster than marking
    fn sweep_step(&mut self, pool: &mut ObjectPool, _max_work: usize) -> usize {
        // Sweep all pools in one step (much faster than incremental)
        let collected = self.sweep_pools(pool);
        self.stats.objects_collected += collected;

        // Sweeping done - transition to finished
        self.state = GcState::Pause;
        self.finish_cycle();

        collected
    }

    /// Sweep all pools directly
    fn sweep_pools(&mut self, pool: &mut ObjectPool) -> usize {
        let mut collected = 0;

        // Sweep tables
        let mut dead_tables: Vec<u32> = Vec::with_capacity(64);
        for (id, table) in pool.tables.iter() {
            if !table.header.is_fixed() && table.header.is_white() {
                dead_tables.push(id);
            }
        }
        for id in dead_tables {
            pool.tables.free(id);
            self.total_bytes = self.total_bytes.saturating_sub(256);
            self.record_deallocation(256);
            collected += 1;
        }

        // Sweep functions
        let mut dead_funcs: Vec<u32> = Vec::with_capacity(64);
        for (id, func) in pool.functions.iter() {
            if !func.header.is_fixed() && func.header.is_white() {
                dead_funcs.push(id);
            }
        }
        for id in dead_funcs {
            pool.functions.free(id);
            self.total_bytes = self.total_bytes.saturating_sub(128);
            self.record_deallocation(128);
            collected += 1;
        }

        // Sweep upvalues
        let mut dead_upvals: Vec<u32> = Vec::with_capacity(64);
        for (id, upval) in pool.upvalues.iter() {
            if !upval.header.is_fixed() && upval.header.is_white() {
                dead_upvals.push(id);
            }
        }
        for id in dead_upvals {
            pool.upvalues.free(id);
            self.record_deallocation(64);
            collected += 1;
        }

        // Sweep strings
        let mut dead_strings: Vec<u32> = Vec::with_capacity(64);
        for (id, string) in pool.strings.iter() {
            if !string.header.is_fixed() && string.header.is_white() {
                dead_strings.push(id);
            }
        }
        for id in dead_strings {
            pool.strings.free(id);
            self.total_bytes = self.total_bytes.saturating_sub(64);
            self.record_deallocation(64);
            collected += 1;
        }

        // Sweep threads
        let mut dead_threads: Vec<u32> = Vec::with_capacity(8);
        for (id, thread) in pool.threads.iter() {
            if !thread.header.is_fixed() && thread.header.is_white() {
                dead_threads.push(id);
            }
        }
        for id in dead_threads {
            pool.threads.free(id);
            self.record_deallocation(512);
            collected += 1;
        }

        collected
    }

    /// Finish the GC cycle
    fn finish_cycle(&mut self) {
        // Flip white bit for next cycle
        self.current_white ^= 1;

        // Set debt based on memory and pause factor
        let estimate = self.total_bytes;
        let threshold = (estimate as isize * self.gc_pause as isize) / 100;
        self.gc_debt = self.total_bytes as isize - threshold;
    }

    // ============ Generational GC Methods ============

    /// Set debt for next minor collection
    /// Minor GC happens when memory grows by gen_minor_mul%
    fn set_minor_debt(&mut self) {
        let debt = -((self.total_bytes / 100) as isize * self.gen_minor_mul as isize);
        self.gc_debt = debt;
    }

    /// Set pause for major collection (like Lua's setpause)
    fn set_pause(&mut self) {
        let estimate = self.gc_estimate.max(self.total_bytes);
        let threshold = (estimate as isize * self.gc_pause as isize) / 100;
        self.gc_debt = self.total_bytes as isize - threshold;
    }

    /// Minor collection - only collect young generation
    /// Like Lua's youngcollection
    fn young_collection(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.stats.collection_count += 1;
        self.stats.minor_collections += 1;

        // Clear gray lists
        self.gray.clear();
        self.grayagain.clear();

        // CRITICAL: Make all young objects white before marking
        // This ensures unmarked objects will be considered dead
        for gc_id in &self.young_list {
            self.make_white(*gc_id, pool);
        }

        // Mark roots
        for value in roots {
            self.mark_value(value, pool);
        }

        // Mark touched old objects (they may point to young objects)
        for gc_id in std::mem::take(&mut self.touched_list) {
            self.mark_object_gen(gc_id, pool);
        }

        // Propagate marks
        while let Some(gc_id) = self.gray.pop() {
            self.mark_one(gc_id, pool);
        }

        // Process grayagain
        while let Some(gc_id) = self.grayagain.pop() {
            self.mark_one(gc_id, pool);
        }

        // Clear weak table entries before sweep
        self.clear_weak_tables(pool);

        // Sweep young objects and age them
        let collected = self.sweep_young(pool);
        self.stats.objects_collected += collected;

        // Flip white for next cycle
        self.current_white ^= 1;
    }

    /// Mark an object for generational GC
    fn mark_object_gen(&mut self, gc_id: GcId, pool: &mut ObjectPool) {
        match gc_id.gc_type() {
            GcObjectType::Table => {
                if let Some(t) = pool.tables.get_mut(gc_id.index()) {
                    if t.header.is_white() {
                        t.header.make_gray();
                        self.gray.push(gc_id);
                    }
                }
            }
            GcObjectType::Function => {
                if let Some(f) = pool.functions.get_mut(gc_id.index()) {
                    if f.header.is_white() {
                        f.header.make_gray();
                        self.gray.push(gc_id);
                    }
                }
            }
            GcObjectType::Upvalue => {
                if let Some(u) = pool.upvalues.get_mut(gc_id.index()) {
                    if u.header.is_white() {
                        u.header.make_gray();
                        self.gray.push(gc_id);
                    }
                }
            }
            GcObjectType::Thread => {
                if let Some(t) = pool.threads.get_mut(gc_id.index()) {
                    if t.header.is_white() {
                        t.header.make_gray();
                        self.gray.push(gc_id);
                    }
                }
            }
            GcObjectType::String => {
                if let Some(s) = pool.strings.get_mut(gc_id.index()) {
                    s.header.make_black();
                }
            }
            GcObjectType::Userdata => {}
        }
    }

    /// Sweep young objects: delete dead, age survivors
    fn sweep_young(&mut self, pool: &mut ObjectPool) -> usize {
        let mut collected = 0;
        let mut new_young = Vec::with_capacity(self.young_list.len());

        for gc_id in std::mem::take(&mut self.young_list) {
            let (is_alive, age) = self.get_object_age(gc_id, pool);

            if !is_alive {
                // Dead object - free it
                self.free_object(gc_id, pool);
                collected += 1;
            } else {
                // Alive - advance age
                let new_age = match age {
                    G_NEW => G_SURVIVAL,
                    G_SURVIVAL => G_OLD1,
                    _ => age,
                };

                self.set_object_age(gc_id, new_age, pool);

                // Keep in young list if still young, otherwise it graduates
                if new_age <= G_SURVIVAL {
                    new_young.push(gc_id);
                } else {
                    self.stats.promoted_objects += 1;
                }

                // Make white for next cycle
                self.make_white(gc_id, pool);
            }
        }

        self.young_list = new_young;
        collected
    }

    /// Get object's age
    fn get_object_age(&self, gc_id: GcId, pool: &ObjectPool) -> (bool, u8) {
        match gc_id.gc_type() {
            GcObjectType::Table => {
                if let Some(t) = pool.tables.get(gc_id.index()) {
                    (!t.header.is_white(), t.header.age())
                } else {
                    (false, G_NEW)
                }
            }
            GcObjectType::Function => {
                if let Some(f) = pool.functions.get(gc_id.index()) {
                    (!f.header.is_white(), f.header.age())
                } else {
                    (false, G_NEW)
                }
            }
            GcObjectType::Upvalue => {
                if let Some(u) = pool.upvalues.get(gc_id.index()) {
                    (!u.header.is_white(), u.header.age())
                } else {
                    (false, G_NEW)
                }
            }
            GcObjectType::Thread => {
                if let Some(t) = pool.threads.get(gc_id.index()) {
                    (!t.header.is_white(), t.header.age())
                } else {
                    (false, G_NEW)
                }
            }
            GcObjectType::String => {
                if let Some(s) = pool.strings.get(gc_id.index()) {
                    (!s.header.is_white(), s.header.age())
                } else {
                    (false, G_NEW)
                }
            }
            GcObjectType::Userdata => (true, G_OLD),
        }
    }

    /// Set object's age
    fn set_object_age(&self, gc_id: GcId, age: u8, pool: &mut ObjectPool) {
        match gc_id.gc_type() {
            GcObjectType::Table => {
                if let Some(t) = pool.tables.get_mut(gc_id.index()) {
                    t.header.set_age(age);
                }
            }
            GcObjectType::Function => {
                if let Some(f) = pool.functions.get_mut(gc_id.index()) {
                    f.header.set_age(age);
                }
            }
            GcObjectType::Upvalue => {
                if let Some(u) = pool.upvalues.get_mut(gc_id.index()) {
                    u.header.set_age(age);
                }
            }
            GcObjectType::Thread => {
                if let Some(t) = pool.threads.get_mut(gc_id.index()) {
                    t.header.set_age(age);
                }
            }
            GcObjectType::String => {
                if let Some(s) = pool.strings.get_mut(gc_id.index()) {
                    s.header.set_age(age);
                }
            }
            GcObjectType::Userdata => {}
        }
    }

    /// Full generational collection - like Lua's fullgen
    fn full_gen(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) -> usize {
        self.stats.major_collections += 1;

        // Do a full mark-sweep
        self.clear_marks(pool);
        self.mark_roots(roots, pool);
        // Clear weak table entries before sweep
        self.clear_weak_tables(pool);
        let collected = self.sweep(pool);

        // Reset generational state
        self.gc_estimate = self.total_bytes;
        self.young_list.clear();
        self.touched_list.clear();

        // Make all surviving objects old
        self.make_all_old(pool);

        self.stats.objects_collected += collected;
        collected
    }

    /// Make all surviving objects old (for entering generational mode)
    fn make_all_old(&self, pool: &mut ObjectPool) {
        for (_id, t) in pool.tables.iter_mut() {
            if !t.header.is_fixed() {
                t.header.set_age(G_OLD);
                t.header.make_black();
            }
        }
        for (_id, f) in pool.functions.iter_mut() {
            if !f.header.is_fixed() {
                f.header.set_age(G_OLD);
                f.header.make_black();
            }
        }
        for (_id, u) in pool.upvalues.iter_mut() {
            if !u.header.is_fixed() {
                u.header.set_age(G_OLD);
                u.header.make_black();
            }
        }
        for (_id, t) in pool.threads.iter_mut() {
            if !t.header.is_fixed() {
                t.header.set_age(G_OLD);
                t.header.make_black();
            }
        }
        for (_id, s) in pool.strings.iter_mut() {
            if !s.header.is_fixed() {
                s.header.set_age(G_OLD);
                s.header.make_black();
            }
        }
    }

    /// Handle bad collection - step through full GC
    fn step_gen_full(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        let last_atomic = self.last_atomic;

        // Do a full collection
        let new_atomic = self.full_gen(roots, pool);

        // Check if this was a good collection
        if new_atomic < last_atomic + (last_atomic / 8) {
            // Good - return to generational mode
            self.last_atomic = 0;
            self.set_minor_debt();
        } else {
            // Still bad
            self.last_atomic = new_atomic;
            self.set_pause();
        }
    }

    // ============ Write Barriers for Generational GC ============

    /// Forward barrier: when black object 'from' points to white object 'to'
    /// Mark 'to' and possibly make it old
    pub fn barrier_forward_gen(&mut self, from_id: GcId, to_id: GcId, pool: &mut ObjectPool) {
        if self.gc_kind != GcKind::Generational {
            return;
        }

        // Check if 'from' is old
        let from_is_old = self.is_object_old(from_id, pool);

        if from_is_old {
            // Mark the target object and make it OLD0
            // This ensures it won't be collected and will age properly
            self.mark_object_gen(to_id, pool);
            self.set_object_age(to_id, G_OLD0, pool);
        }
    }

    /// Back barrier: when old object 'obj' is modified to point to young object
    /// Mark 'obj' as touched so it will be revisited in minor collection
    pub fn barrier_back_gen(&mut self, obj_id: GcId, pool: &mut ObjectPool) {
        if self.gc_kind != GcKind::Generational {
            return;
        }

        let age = match obj_id.gc_type() {
            GcObjectType::Table => pool.tables.get(obj_id.index()).map(|t| t.header.age()),
            GcObjectType::Function => pool.functions.get(obj_id.index()).map(|f| f.header.age()),
            GcObjectType::Thread => pool.threads.get(obj_id.index()).map(|t| t.header.age()),
            _ => None,
        };

        if let Some(age) = age {
            if age >= G_OLD0 && age != G_TOUCHED1 {
                // Mark as touched and add to touched list
                self.set_object_age(obj_id, G_TOUCHED1, pool);
                self.touched_list.push(obj_id);
            }
        }
    }

    /// Check if an object is old
    fn is_object_old(&self, gc_id: GcId, pool: &ObjectPool) -> bool {
        match gc_id.gc_type() {
            GcObjectType::Table => pool
                .tables
                .get(gc_id.index())
                .map(|t| t.header.age() >= G_OLD0)
                .unwrap_or(false),
            GcObjectType::Function => pool
                .functions
                .get(gc_id.index())
                .map(|f| f.header.age() >= G_OLD0)
                .unwrap_or(false),
            GcObjectType::Upvalue => pool
                .upvalues
                .get(gc_id.index())
                .map(|u| u.header.age() >= G_OLD0)
                .unwrap_or(false),
            GcObjectType::Thread => pool
                .threads
                .get(gc_id.index())
                .map(|t| t.header.age() >= G_OLD0)
                .unwrap_or(false),
            GcObjectType::String => pool
                .strings
                .get(gc_id.index())
                .map(|s| s.header.age() >= G_OLD0)
                .unwrap_or(false),
            GcObjectType::Userdata => true,
        }
    }

    /// Main collection - mark and sweep using allgc list
    /// Like Lua's full GC cycle
    pub fn collect(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) -> usize {
        self.stats.collection_count += 1;
        self.stats.major_collections += 1;

        // Phase 1: Clear all marks (only for tracked objects)
        self.clear_marks(pool);

        // Phase 2: Mark from roots
        self.mark_roots(roots, pool);

        // Phase 3: Clear weak table entries (before sweep!)
        self.clear_weak_tables(pool);

        // Phase 4: Sweep (only traverse allgc, not entire pools!)
        let collected = self.sweep(pool);

        // Like Lua's setpause: set debt based on memory and pause factor
        // gc_pause = 200 means wait until memory doubles (200% of current)
        // debt = current_memory - (estimate * pause / 100)
        // Since estimate ≈ current_memory after GC, debt becomes negative
        let estimate = self.total_bytes;
        let threshold = (estimate as isize * self.gc_pause as isize) / 100;
        self.gc_debt = self.total_bytes as isize - threshold;

        self.stats.objects_collected += collected;
        collected
    }

    /// Clear marks by iterating pools directly (no allgc needed)
    fn clear_marks(&self, pool: &mut ObjectPool) {
        // Clear tables
        for (_id, table) in pool.tables.iter_mut() {
            if !table.header.is_fixed() {
                table.header.make_white(0);
            }
        }

        // Clear functions
        for (_id, func) in pool.functions.iter_mut() {
            if !func.header.is_fixed() {
                func.header.make_white(0);
            }
        }

        // Clear upvalues
        for (_id, upval) in pool.upvalues.iter_mut() {
            if !upval.header.is_fixed() {
                upval.header.make_white(0);
            }
        }

        // Clear threads
        for (_id, thread) in pool.threads.iter_mut() {
            if !thread.header.is_fixed() {
                thread.header.make_white(0);
            }
        }

        // Clear strings (but leave interned strings fixed)
        for (_id, string) in pool.strings.iter_mut() {
            if !string.header.is_fixed() {
                string.header.make_white(0);
            }
        }
    }

    /// Mark phase - traverse from roots
    /// Uses a worklist algorithm to avoid recursion and handle borrowing correctly
    fn mark_roots(&self, roots: &[LuaValue], pool: &mut ObjectPool) {
        let mut worklist: Vec<LuaValue> = roots.to_vec();
        // Track which fixed tables we've already traversed (to avoid infinite loops)
        let mut traversed_fixed: std::collections::HashSet<u32> = std::collections::HashSet::new();

        while let Some(value) = worklist.pop() {
            match value.kind() {
                crate::lua_value::LuaValueKind::Table => {
                    if let Some(id) = value.as_table_id() {
                        // First pass: collect info without mutating
                        let (should_traverse, is_fixed, entries, mt_value, weak_mode) = {
                            if let Some(table) = pool.tables.get(id.0) {
                                let is_fixed = table.header.is_fixed();
                                let should = if is_fixed {
                                    !traversed_fixed.contains(&id.0)
                                } else {
                                    table.header.is_white()
                                };

                                if should {
                                    let entries = table.data.iter_all();
                                    let mt = table.data.get_metatable();

                                    // Check weak mode from metatable
                                    let weak = if let Some(mt_id) = mt.and_then(|v| v.as_table_id())
                                    {
                                        if let Some(mt_table) = pool.tables.get(mt_id.0) {
                                            self.get_weak_mode(mt_table, pool)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };

                                    (true, is_fixed, entries, mt, weak)
                                } else {
                                    (false, is_fixed, vec![], None, None)
                                }
                            } else {
                                (false, false, vec![], None, None)
                            }
                        };

                        if should_traverse {
                            // Mark the traversal
                            if is_fixed {
                                traversed_fixed.insert(id.0);
                            } else {
                                if let Some(table) = pool.tables.get_mut(id.0) {
                                    table.header.make_black();
                                }
                            }

                            let (weak_keys, weak_values) = weak_mode.unwrap_or((false, false));

                            // Add table contents to worklist (skip weak references)
                            for (k, v) in entries {
                                if !weak_keys {
                                    worklist.push(k);
                                }
                                if !weak_values {
                                    worklist.push(v);
                                }
                            }
                            if let Some(mt) = mt_value {
                                worklist.push(mt);
                            }
                        }
                    }
                }
                crate::lua_value::LuaValueKind::Function => {
                    if let Some(id) = value.as_function_id() {
                        // First, collect data we need without holding mutable borrow
                        let (should_mark, upvalue_ids, constants) = {
                            if let Some(func) = pool.functions.get(id.0) {
                                if func.header.is_white() {
                                    // C closures don't have constants
                                    let consts = func
                                        .chunk()
                                        .map(|c| c.constants.clone())
                                        .unwrap_or_default();
                                    (true, func.upvalues.clone(), consts)
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
                                func.header.make_black();
                            }

                            // Mark upvalues separately
                            for upval_id in upvalue_ids {
                                if let Some(upval) = pool.upvalues.get_mut(upval_id.0) {
                                    if upval.header.is_white() {
                                        upval.header.make_black();
                                        if !upval.is_open {
                                            worklist.push(upval.closed_value);
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
                                if thread.header.is_white() {
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
                                thread.header.make_black();
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
                            string.header.make_black();
                        }
                    }
                }
                _ => {} // Numbers, booleans, nil, CFunction - no marking needed
            }
        }
    }

    /// Check if a LuaValue points to a dead (white/unmarked) GC object
    fn is_value_dead(&self, value: &LuaValue, pool: &ObjectPool) -> bool {
        use crate::lua_value::LuaValueKind;
        match value.kind() {
            LuaValueKind::Table => {
                if let Some(id) = value.as_table_id() {
                    if let Some(t) = pool.tables.get(id.0) {
                        return !t.header.is_fixed() && t.header.is_white();
                    }
                }
                false
            }
            LuaValueKind::Function => {
                if let Some(id) = value.as_function_id() {
                    if let Some(f) = pool.functions.get(id.0) {
                        return !f.header.is_fixed() && f.header.is_white();
                    }
                }
                false
            }
            LuaValueKind::Thread => {
                if let Some(id) = value.as_thread_id() {
                    if let Some(t) = pool.threads.get(id.0) {
                        return !t.header.is_fixed() && t.header.is_white();
                    }
                }
                false
            }
            LuaValueKind::String => {
                if let Some(id) = value.as_string_id() {
                    if let Some(s) = pool.strings.get(id.0) {
                        return !s.header.is_fixed() && s.header.is_white();
                    }
                }
                false
            }
            // Numbers, booleans, nil, CFunction, Userdata - not GC managed or always live
            _ => false,
        }
    }

    /// Clear weak table entries that point to dead (white) objects
    /// This must be called after marking but before sweeping
    fn clear_weak_tables(&self, pool: &mut ObjectPool) -> usize {
        // Collect tables with weak mode and their entries to remove
        let mut tables_to_clear: Vec<(u32, Vec<LuaValue>)> = Vec::new();
        let mut total_cleared = 0;

        for (id, table) in pool.tables.iter() {
            // Check if this table has a metatable with __mode
            if let Some(mt_id) = table.data.get_metatable().and_then(|v| v.as_table_id()) {
                if let Some(mt) = pool.tables.get(mt_id.0) {
                    // Look for __mode key in metatable
                    let mode = self.get_weak_mode(mt, pool);
                    if let Some((weak_keys, weak_values)) = mode {
                        // Collect keys to remove
                        let mut keys_to_remove = Vec::new();
                        for (key, value) in table.data.iter_all() {
                            let key_dead = weak_keys && self.is_value_dead(&key, pool);
                            let value_dead = weak_values && self.is_value_dead(&value, pool);

                            if key_dead || value_dead {
                                keys_to_remove.push(key);
                            }
                        }
                        if !keys_to_remove.is_empty() {
                            tables_to_clear.push((id, keys_to_remove));
                        }
                    }
                }
            }
        }

        // Now actually remove the entries
        for (table_id, keys) in tables_to_clear {
            if let Some(table) = pool.tables.get_mut(table_id) {
                for key in keys {
                    table.data.raw_set(key, LuaValue::nil());
                    total_cleared += 1;
                }
            }
        }

        total_cleared
    }

    /// Get weak mode from metatable's __mode field
    /// Returns Some((weak_keys, weak_values)) if __mode is set
    fn get_weak_mode(&self, metatable: &GcTable, pool: &ObjectPool) -> Option<(bool, bool)> {
        // Find __mode key - it should be a string "k", "v", or "kv" (or "vk")
        for n in metatable.data.nodes.iter() {
            let key = &n.key;
            // Check if key is the string "__mode"
            if let Some(key_str_id) = key.as_string_id() {
                if key_str_id == pool.tm_mode {
                    let value = &n.value;
                    // Found __mode, now check the value
                    if let Some(val_str_id) = value.as_string_id() {
                        if let Some(val_str) = pool.strings.get(val_str_id.0) {
                            let mode_str = val_str.data.as_str();
                            let weak_keys = mode_str.contains('k');
                            let weak_values = mode_str.contains('v');
                            if weak_keys || weak_values {
                                return Some((weak_keys, weak_values));
                            }
                        }
                    }
                    return None;
                }
            }
        }

        None
    }

    /// Sweep phase - iterate pools directly instead of allgc
    /// This is much faster for allocation (no allgc.push) at cost of sweep traversal
    fn sweep(&mut self, pool: &mut ObjectPool) -> usize {
        let mut collected = 0;

        // Sweep tables
        let mut dead_tables: Vec<u32> = Vec::with_capacity(64);
        for (id, table) in pool.tables.iter() {
            if !table.header.is_fixed() && table.header.is_white() {
                dead_tables.push(id);
            }
        }
        for id in dead_tables {
            pool.tables.free(id);
            self.record_deallocation(256);
            collected += 1;
        }

        // Sweep functions
        let mut dead_funcs: Vec<u32> = Vec::with_capacity(64);
        for (id, func) in pool.functions.iter() {
            if !func.header.is_fixed() && func.header.is_white() {
                dead_funcs.push(id);
            }
        }
        for id in dead_funcs {
            pool.functions.free(id);
            self.record_deallocation(128);
            collected += 1;
        }

        // Sweep upvalues
        let mut dead_upvals: Vec<u32> = Vec::with_capacity(64);
        for (id, upval) in pool.upvalues.iter() {
            if !upval.header.is_fixed() && upval.header.is_white() {
                dead_upvals.push(id);
            }
        }
        for id in dead_upvals {
            pool.upvalues.free(id);
            self.record_deallocation(64);
            collected += 1;
        }

        // Sweep strings - but leave interned strings (short strings are usually fixed)
        let mut dead_strings: Vec<u32> = Vec::with_capacity(64);
        for (id, string) in pool.strings.iter() {
            if !string.header.is_fixed() && string.header.is_white() {
                dead_strings.push(id);
            }
        }
        for id in dead_strings {
            pool.strings.free(id);
            self.record_deallocation(64);
            collected += 1;
        }

        // Sweep threads
        let mut dead_threads: Vec<u32> = Vec::with_capacity(8);
        for (id, thread) in pool.threads.iter() {
            if !thread.header.is_fixed() && thread.header.is_white() {
                dead_threads.push(id);
            }
        }
        for id in dead_threads {
            pool.threads.free(id);
            self.record_deallocation(512);
            collected += 1;
        }

        collected
    }

    /// Get marked (not white) and fixed state for an object
    #[allow(unused)]
    #[inline]
    fn get_object_state(&self, gc_id: GcId, pool: &ObjectPool) -> (bool, bool) {
        match gc_id.gc_type() {
            GcObjectType::Table => {
                if let Some(t) = pool.tables.get(gc_id.index()) {
                    (!t.header.is_white(), t.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcObjectType::Function => {
                if let Some(f) = pool.functions.get(gc_id.index()) {
                    (!f.header.is_white(), f.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcObjectType::Upvalue => {
                if let Some(u) = pool.upvalues.get(gc_id.index()) {
                    (!u.header.is_white(), u.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcObjectType::Thread => {
                if let Some(t) = pool.threads.get(gc_id.index()) {
                    (!t.header.is_white(), t.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcObjectType::String => {
                if let Some(s) = pool.strings.get(gc_id.index()) {
                    (!s.header.is_white(), s.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcObjectType::Userdata => (true, true), // Userdata uses Rc, always "alive"
        }
    }

    /// Free an object from its pool
    #[inline]
    fn free_object(&mut self, gc_id: GcId, pool: &mut ObjectPool) {
        match gc_id.gc_type() {
            GcObjectType::Table => {
                pool.tables.free(gc_id.index());
                self.record_deallocation(256);
            }
            GcObjectType::Function => {
                pool.functions.free(gc_id.index());
                self.record_deallocation(128);
            }
            GcObjectType::Upvalue => {
                pool.upvalues.free(gc_id.index());
                self.record_deallocation(64);
            }
            GcObjectType::Thread => {
                pool.threads.free(gc_id.index());
                self.record_deallocation(512);
            }
            GcObjectType::String => {
                pool.strings.free(gc_id.index());
                self.record_deallocation(64);
            }
            GcObjectType::Userdata => {} // Rc handles this
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
            GcObjectType::Upvalue => 64,
            GcObjectType::Thread => 512,
            GcObjectType::Userdata => 32,
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
