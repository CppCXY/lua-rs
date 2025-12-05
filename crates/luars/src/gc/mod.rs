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
    gc_estimate: usize, // Estimate of memory in use after major collection

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
            gc_kind: GcKind::Generational, // Default to generational mode like Lua 5.4
            sweep_index: 0,
            propagate_work: 0,
            gc_pause: 200,      // Like Lua: 200 = wait until memory doubles
            gen_minor_mul: 25,  // Minor GC when memory grows 25%
            gen_major_mul: 100, // Major GC when memory grows 100% since last major
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

    /// Incremental GC step - original incremental mode
    fn inc_step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        const WORK_PER_STEP: usize = 4096;
        let mut work = 0;

        // State machine for incremental GC
        loop {
            match self.state {
                GcState::Pause => {
                    // Start new cycle: mark roots and transition to propagate
                    self.start_cycle(roots, pool);
                    self.state = GcState::Propagate;
                    work += 100; // Small fixed cost
                }

                GcState::Propagate => {
                    // Incremental marking: process some gray objects
                    let marked = self.propagate_step(pool, WORK_PER_STEP - work);
                    work += marked;

                    if self.gray.is_empty() && self.grayagain.is_empty() {
                        // All marking done, go to atomic phase
                        self.state = GcState::Atomic;
                    }
                }

                GcState::Atomic => {
                    // Atomic phase - must finish marking (like Lua's atomic)
                    // Process any grayagain objects
                    while let Some(gc_id) = self.grayagain.pop() {
                        self.mark_one(gc_id, pool);
                    }
                    // Start sweep
                    self.sweep_index = 0;
                    self.state = GcState::Sweep;
                    work += 50;
                }

                GcState::Sweep => {
                    // Complete sweep in one step (pools are iterated directly)
                    let swept = self.sweep_step(pool, WORK_PER_STEP - work);
                    work += swept;
                    // sweep_step handles state transition and finish_cycle
                    break;
                }
            }

            // Check if we've done enough work for this step
            if work >= WORK_PER_STEP {
                break;
            }
        }

        // Reduce debt by work done (convert work to "bytes paid off")
        self.gc_debt -= (work as isize) * 2;
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
                if let Some(table) = pool.tables.get_mut(gc_id.index()) {
                    if table.header.is_gray() {
                        table.header.make_black();
                        work += table.data.len();

                        // Collect references to mark
                        let refs: Vec<LuaValue> = table
                            .data
                            .iter_all()
                            .into_iter()
                            .flat_map(|(k, v)| [k, v])
                            .collect();
                        let mt = table.data.get_metatable();

                        // Mark references
                        for v in refs {
                            self.mark_value(&v, pool);
                        }
                        if let Some(mt) = mt {
                            self.mark_value(&mt, pool);
                        }
                    }
                }
            }
            GcObjectType::Function => {
                if let Some(func) = pool.functions.get(gc_id.index()) {
                    let upvalue_ids = func.upvalues.clone();
                    let constants = func.chunk.constants.clone();

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
                if let Some(upval) = pool.upvalues.get_mut(gc_id.index()) {
                    if upval.header.is_gray() {
                        upval.header.make_black();
                        if let UpvalueState::Closed(v) = upval.state {
                            self.mark_value(&v, pool);
                        }
                    }
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

        // Sweep strings
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

        // Phase 3: Sweep (only traverse allgc, not entire pools!)
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

        while let Some(value) = worklist.pop() {
            match value.kind() {
                crate::lua_value::LuaValueKind::Table => {
                    if let Some(id) = value.as_table_id() {
                        if let Some(table) = pool.tables.get_mut(id.0) {
                            if table.header.is_white() {
                                table.header.make_black();
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
                                if func.header.is_white() {
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
                                func.header.make_black();
                            }

                            // Mark upvalues separately
                            for upval_id in upvalue_ids {
                                if let Some(upval) = pool.upvalues.get_mut(upval_id.0) {
                                    if upval.header.is_white() {
                                        upval.header.make_black();
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
