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

mod gc_id;
mod gc_object;
mod object_pool;

use crate::lua_value::{LuaValue, LuaValueKind};
pub use gc_id::*;
pub use gc_object::*;
pub use object_pool::*;

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
    // Gray list for incremental marking
    gray: Vec<GcId>,
    grayagain: Vec<GcId>,
    // Lua 5.4 GC debt mechanism
    pub(crate) gc_debt: isize,
    pub(crate) total_bytes: usize,

    // GC state
    state: GcState,
    current_white: u8, // 0 or 1, flips each cycle

    // Incremental sweep state
    sweep_index: usize,    // Current position in sweep phase
    propagate_work: usize, // Work done in propagate phase

    // GC parameters
    gc_pause: usize,   // Pause parameter (default 200 = 200%)
    gc_stepmul: usize, // Step multiplier (default 100)

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
            // Lua uses -GCSTEPSIZE (8KB by default)
            gc_debt: -(8 * 1024), // 8KB credit before first GC
            total_bytes: 0,
            state: GcState::Pause,
            current_white: 0,
            sweep_index: 0,
            propagate_work: 0,
            gc_pause: 200,   // Like Lua: 200 = wait until memory doubles
            gc_stepmul: 100, // Step multiplier
            check_counter: 0,
            check_interval: 1, // Check every time (Lua doesn't use interval)
            stats: GCStats::default(),
        }
    }

    /// Register a new object for GC tracking (no-op, pools are scanned directly)
    #[inline(always)]
    pub fn track_object(&mut self, _gc_id: GcId, size: usize) {
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

    /// Perform GC step - like Lua's luaC_step
    /// This does incremental work instead of full collection
    pub fn step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // Like Lua: run GC when debt > 0
        if self.gc_debt <= 0 {
            return;
        }

        // Calculate work budget for this step
        // Like Lua's gettotalbytes(g) / WORK2MEM
        const WORK_PER_STEP: usize = 4096; // Do 4KB worth of work per step
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
            GcType::Table => {
                if let Some(t) = pool.tables.get_mut(gc_id.index()) {
                    if !t.header.is_fixed() {
                        t.header.make_white(self.current_white);
                    }
                }
            }
            GcType::Function => {
                if let Some(f) = pool.functions.get_mut(gc_id.index()) {
                    if !f.header.is_fixed() {
                        f.header.make_white(self.current_white);
                    }
                }
            }
            GcType::Upvalue => {
                if let Some(u) = pool.upvalues.get_mut(gc_id.index()) {
                    if !u.header.is_fixed() {
                        u.header.make_white(self.current_white);
                    }
                }
            }
            GcType::Thread => {
                if let Some(t) = pool.threads.get_mut(gc_id.index()) {
                    if !t.header.is_fixed() {
                        t.header.make_white(self.current_white);
                    }
                }
            }
            GcType::String => {
                if let Some(s) = pool.strings.get_mut(gc_id.index()) {
                    if !s.header.is_fixed() {
                        s.header.make_white(self.current_white);
                    }
                }
            }
            GcType::Userdata => {}
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
            GcType::Table => {
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
            GcType::Function => {
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
            GcType::Upvalue => {
                if let Some(upval) = pool.upvalues.get_mut(gc_id.index()) {
                    if upval.header.is_gray() {
                        upval.header.make_black();
                        if let UpvalueState::Closed(v) = upval.state {
                            self.mark_value(&v, pool);
                        }
                    }
                }
            }
            GcType::Thread => {
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
            GcType::String => {
                // Strings are leaves, just make black
                if let Some(s) = pool.strings.get_mut(gc_id.index()) {
                    s.header.make_black();
                }
            }
            GcType::Userdata => {}
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
    #[inline]
    fn get_object_state(&self, gc_id: GcId, pool: &ObjectPool) -> (bool, bool) {
        match gc_id.gc_type() {
            GcType::Table => {
                if let Some(t) = pool.tables.get(gc_id.index()) {
                    (!t.header.is_white(), t.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcType::Function => {
                if let Some(f) = pool.functions.get(gc_id.index()) {
                    (!f.header.is_white(), f.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcType::Upvalue => {
                if let Some(u) = pool.upvalues.get(gc_id.index()) {
                    (!u.header.is_white(), u.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcType::Thread => {
                if let Some(t) = pool.threads.get(gc_id.index()) {
                    (!t.header.is_white(), t.header.is_fixed())
                } else {
                    (false, false)
                }
            }
            GcType::String => {
                if let Some(s) = pool.strings.get(gc_id.index()) {
                    (!s.header.is_white(), s.header.is_fixed())
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
