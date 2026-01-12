// Garbage Collector for Lua 5.5
//
// This is a faithful port of Lua 5.5's garbage collector from lgc.c
// Supporting three modes:
// - KGC_INC: Incremental mark-sweep (traditional)
// - KGC_GENMINOR: Generational mode doing minor collections
// - KGC_GENMAJOR: Generational mode doing major collections (uses incremental temporarily)
//
// Key structures from global_State:
// - GCdebt: Debt-based triggering (when debt > 0, run GC)
// - GCtotalbytes: Total allocated bytes + debt
// - GCmarked: Bytes marked in current cycle
// - GCmajorminor: Aux counter for major/minor mode shifts
//
// Object ages (generational mode):
// - G_NEW (0): Created in current cycle
// - G_SURVIVAL (1): Survived one collection
// - G_OLD0 (2): Marked old by forward barrier
// - G_OLD1 (3): First cycle as old
// - G_OLD (4): Really old
// - G_TOUCHED1 (5): Old touched this cycle
// - G_TOUCHED2 (6): Old touched in previous cycle
//
// GC States:
// - GCSpause: Between cycles
// - GCSpropagate: Marking objects
// - GCSenteratomic: Entering atomic phase
// - GCSatomic: Atomic phase (not directly used, implicit in enteratomic)
// - GCSswpallgc: Sweeping regular objects
// - GCSswpfinobj: Sweeping objects with finalizers
// - GCSswptobefnz: Sweeping objects to be finalized
// - GCSswpend: Sweep finished
// - GCScallfin: Calling finalizers
//
// Tri-color invariant: Black objects cannot point to white objects

mod const_string;
mod gc_id;
mod gc_object;
mod object_pool;

use crate::lua_value::{LuaValue, LuaValueKind, Chunk};
pub use gc_id::*;
pub use gc_object::*;
pub use object_pool::*;

// GC Parameters (from lua.h)
pub const PAUSE: usize = 0; // Pause between GC cycles (default 200%)
pub const STEPMUL: usize = 1; // GC speed multiplier (default 200)
pub const STEPSIZE: usize = 2; // Step size in KB (default 13KB)
pub const MINORMUL: usize = 3; // Minor collection multiplier (default 20%)
pub const MINORMAJOR: usize = 4; // Shift from minor to major (default 100%)
pub const MAJORMINOR: usize = 5; // Shift from major to minor (default 100%)
pub const GCPARAM_COUNT: usize = 6;

// Default GC parameters (from luaconf.h)
const DEFAULT_PAUSE: i32 = 200; // 200%
const DEFAULT_STEPMUL: i32 = 200; // 200%
const DEFAULT_STEPSIZE: i32 = 13; // 13 KB
const DEFAULT_MINORMUL: i32 = 20; // 20%
const DEFAULT_MINORMAJOR: i32 = 100; // 100%
const DEFAULT_MAJORMINOR: i32 = 100; // 100%

/// GC mode (from lgc.h)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcKind {
    Inc = 0,      // KGC_INC - Incremental mode
    GenMinor = 1, // KGC_GENMINOR - Generational minor collections
    GenMajor = 2, // KGC_GENMAJOR - Generational major collections (temporary inc mode)
}

/// Object age for generational GC (from lgc.h)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GcAge {
    New = 0,      // G_NEW - created in current cycle
    Survival = 1, // G_SURVIVAL - created in previous cycle
    Old0 = 2,     // G_OLD0 - marked old by forward barrier in this cycle
    Old1 = 3,     // G_OLD1 - first full cycle as old
    Old = 4,      // G_OLD - really old object (not to be visited)
    Touched1 = 5, // G_TOUCHED1 - old object touched this cycle
    Touched2 = 6, // G_TOUCHED2 - old object touched in previous cycle
}

impl GcAge {
    pub fn is_old(self) -> bool {
        self as u8 > GcAge::Survival as u8
    }
}

/// GC color for tri-color marking (from lgc.h)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcColor {
    White0 = 0, // object is white (type 0)
    White1 = 1, // object is white (type 1)
    Gray = 2,   // object is gray (marked but not scanned)
    Black = 3,  // object is black (fully marked)
}

/// GC state machine (from lgc.h)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcState {
    Propagate = 0,   // GCSpropagate
    EnterAtomic = 1, // GCSenteratomic
    Atomic = 2,      // GCSatomic (not used directly)
    SwpAllGc = 3,    // GCSswpallgc - sweep regular objects
    SwpFinObj = 4,   // GCSswpfinobj - sweep objects with finalizers
    SwpToBeFnz = 5,  // GCSswptobefnz - sweep objects to be finalized
    SwpEnd = 6,      // GCSswpend - sweep finished
    CallFin = 7,     // GCScallfin - call finalizers
    Pause = 8,       // GCSpause - between cycles
}

impl GcState {
    /// Check if in sweep phase
    pub fn is_sweep_phase(self) -> bool {
        matches!(
            self,
            GcState::SwpAllGc | GcState::SwpFinObj | GcState::SwpToBeFnz | GcState::SwpEnd
        )
    }

    /// Check if must keep invariant (black cannot point to white)
    pub fn keep_invariant(self) -> bool {
        (self as u8) <= (GcState::Atomic as u8)
    }
}

/// Garbage Collector
pub struct GC {
    // === Debt and memory tracking ===
    /// GCdebt from Lua: bytes allocated but not yet "paid for"
    /// When debt > 0, a GC step should run
    pub gc_debt: isize,

    /// GCtotalbytes: total bytes allocated + debt
    pub total_bytes: isize,

    /// GCmarked: bytes marked in current cycle (or bytes added in gen mode)
    pub gc_marked: isize,

    /// GCmajorminor: auxiliary counter for mode shifts in generational GC
    pub gc_majorminor: isize,

    // === GC state ===
    pub gc_state: GcState,
    pub gc_kind: GcKind,

    /// current white color (0 or 1, flips each cycle)
    pub current_white: u8,

    /// is this an emergency collection?
    pub gc_emergency: bool,

    /// stops emergency collections during finalizers
    pub gc_stopem: bool,

    /// GC stopped by user (gcstp in Lua, GCSTPUSR bit)
    pub gc_stopped: bool,

    // === GC parameters (from gcparams[LUA_GCPN]) ===
    pub gc_params: [i32; GCPARAM_COUNT],

    // === Gray lists (for marking) ===
    /// Regular gray objects waiting to be visited
    pub gray: Vec<GcId>,

    /// Objects to be revisited at atomic phase
    pub grayagain: Vec<GcId>,

    // === Generational collector pointers ===
    /// Points to first survival object in allgc list
    pub survival: Option<usize>,

    /// Points to first old1 object
    pub old1: Option<usize>,

    /// Points to first really old object
    pub reallyold: Option<usize>,

    /// Points to first OLD1 object (optimization for markold)
    pub firstold1: Option<usize>,

    // === Statistics ===
    pub stats: GcStats,
}

#[derive(Debug, Clone, Default)]
pub struct GcStats {
    pub collection_count: usize,
    pub minor_collections: usize,
    pub major_collections: usize,
    pub objects_collected: usize,
    pub bytes_allocated: usize,
    pub bytes_freed: usize,
    pub threshold: usize,
    pub young_gen_size: usize,
    pub old_gen_size: usize,
    pub promoted_objects: usize,
}

impl GC {
    pub fn new() -> Self {
        GC {
            gc_debt: 0, // Start with 0 debt, will be set after first allocation
            total_bytes: 0,
            gc_marked: 0,
            gc_majorminor: 0,
            gc_state: GcState::Pause,
            gc_kind: GcKind::GenMinor, // Start in generational mode like Lua 5.5
            current_white: 0,
            gc_emergency: false,
            gc_stopem: false,
            gc_stopped: false,
            gc_params: [
                DEFAULT_PAUSE,      // PAUSE = 0
                DEFAULT_STEPMUL,    // STEPMUL = 1
                DEFAULT_STEPSIZE,   // STEPSIZE = 2
                DEFAULT_MINORMUL,   // MINORMUL = 3
                DEFAULT_MINORMAJOR, // MINORMAJOR = 4
                DEFAULT_MAJORMINOR, // MAJORMINOR = 5
            ],
            gray: Vec::with_capacity(128),
            grayagain: Vec::with_capacity(64),
            survival: None,
            old1: None,
            reallyold: None,
            firstold1: None,
            stats: GcStats::default(),
        }
    }

    /// Change to incremental mode (like minor2inc in Lua 5.5)
    pub fn change_to_incremental_mode(&mut self, pool: &mut ObjectPool) {
        if self.gc_kind == GcKind::Inc {
            return; // Already in incremental mode
        }

        // Save number of live bytes
        self.gc_majorminor = self.gc_marked;

        // Switch mode
        self.gc_kind = GcKind::Inc;

        // Clear generational lists
        self.reallyold = None;
        self.old1 = None;
        self.survival = None;

        // Enter sweep phase (like Lua 5.5's entersweep)
        self.enter_sweep(pool);

        // Set debt for next step
        let stepsize = self.apply_param(STEPSIZE, 100) * 1024;
        self.set_debt(stepsize);
    }

    /// Track a new object allocation (like luaC_newobj in Lua)
    /// This increments debt - when debt becomes positive, GC should run
    /// Also sets object to current white color
    #[inline]
    pub fn track_object(&mut self, gc_id: GcId, size: usize, pool: &mut ObjectPool) {
        // Set object to current white (like Lua 5.5's luaC_newobj)
        match gc_id {
            GcId::TableId(id) => {
                if let Some(t) = pool.tables.get_mut(id.0) {
                    t.header.make_white(self.current_white);
                }
            }
            GcId::FunctionId(id) => {
                if let Some(f) = pool.functions.get_mut(id.0) {
                    f.header.make_white(self.current_white);
                }
            }
            GcId::UpvalueId(id) => {
                if let Some(u) = pool.upvalues.get_mut(id.0) {
                    u.header.make_white(self.current_white);
                }
            }
            GcId::StringId(id) => {
                if let Some(s) = pool.get_string_gc_mut(id) {
                    s.header.make_white(self.current_white);
                }
            }
            GcId::BinaryId(id) => {
                if let Some(b) = pool.binaries.get_mut(id.0) {
                    b.header.make_white(self.current_white);
                }
            }
            GcId::UserdataId(id) => {
                if let Some(u) = pool.userdata.get_mut(id.0) {
                    u.header.make_white(self.current_white);
                }
            }
            GcId::ThreadId(id) => {
                if let Some(t) = pool.threads.get_mut(id.0) {
                    t.header.make_white(self.current_white);
                }
            }
        }
        
        let size_signed = size as isize;
        // Update total_bytes to include both the new allocation AND the debt
        // total_bytes = allocated + debt
        // allocation adds +size to allocated AND +size to debt
        // so total_bytes must increase by 2 * size
        self.total_bytes += size_signed * 2;
        self.gc_debt += size_signed;
        self.stats.bytes_allocated += size;
    }

    /// Record a deallocation
    #[inline]
    pub fn record_deallocation(&mut self, size: usize) {
        let size_signed = size as isize;
        self.total_bytes = self.total_bytes.saturating_sub(size_signed);
        // Do NOT change debt when deallocating during sweep
        // Changing debt would disturb the cycle control
        // self.gc_debt += size_signed; 
        self.stats.bytes_freed += size;
    }

    /// Check if GC should run (debt > 0)
    #[inline]
    pub fn should_collect(&self) -> bool {
        self.gc_debt > 0
    }

    /// Set GC debt (like luaE_setdebt in Lua)
    pub fn set_debt(&mut self, debt: isize) {
        const MAX_DEBT: isize = isize::MAX / 2;
        let real_bytes = self.total_bytes - self.gc_debt;

        let debt = if debt > MAX_DEBT - real_bytes {
            MAX_DEBT - real_bytes
        } else {
            debt
        };

        self.total_bytes = real_bytes + debt;
        self.gc_debt = self.total_bytes - real_bytes;
    }

    /// Apply GC parameter (like applygcparam in Lua)
    fn apply_param(&self, param_idx: usize, value: isize) -> isize {
        let param = self.gc_params[param_idx];
        if param >= 0 {
            (value * param as isize) / 100
        } else {
            // Negative parameters are divided, not multiplied
            (value * 100) / (-param as isize)
        }
    }

    /// Get current GC statistics
    pub fn stats(&self) -> &GcStats {
        &self.stats
    }

    // ============ Core GC Implementation ============

    /// Main GC step function (like luaC_step in Lua 5.5)
    pub fn step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.step_internal(roots, pool, false);
    }

    /// Internal step function with force parameter
    /// If force=true, ignore gc_stopped flag (used by collectgarbage("step"))
    pub fn step_internal(&mut self, roots: &[LuaValue], pool: &mut ObjectPool, force: bool) {
        // Check if GC is stopped by user (unless forced)
        if !force && self.gc_stopped {
            return;
        }
        // If not forced, check debt
        if !force && self.gc_debt <= 0 {
            return; // No need to collect yet
        }

        // Dispatch based on GC mode (like Lua 5.5 luaC_step)
        match self.gc_kind {
            GcKind::Inc | GcKind::GenMajor => {
                self.inc_step(roots, pool);
            }
            GcKind::GenMinor => {
                self.young_collection(roots, pool);
                self.set_minor_debt();
            }
        }
    }

    /// Incremental GC step (like incstep in Lua 5.5)
    fn inc_step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // Calculate step size (like Lua 5.5)
        let stepsize = self.apply_param(STEPSIZE, 100) * 1024; // in bytes (isize)
        
        // Calculate base work from debt, relying on updated accounting
        let debt = self.gc_debt;
        let stepmul = self.apply_param(STEPMUL, 200);

        // Calculate effective work
        // work2do = debt * stepmul / 100
        let effective_debt = debt;
        
        let mut work2do = (effective_debt * stepmul) / 100;
        let fast = work2do == 0; // Special case: do full collection

        // Repeat until enough work is done (like Lua 5.5's do-while loop)
        loop {
            let stres = self.single_step(roots, pool, fast);

            match stres {
                StepResult::MinorMode => {
                    // Returned to minor collections
                    return;
                }
                StepResult::Pause => {
                    // End of cycle (step2pause in Lua)
                    break;
                }
                StepResult::Atomic => {
                    // Atomic step completed (atomicstep in Lua)
                    if !fast {
                        break;
                    }
                    // In fast mode, continue
                }
                StepResult::Work(w) => {
                    // Normal work done
                    work2do -= w;
                }
            }

            // Continue if fast mode or still have work to do
            if !fast && work2do <= 0 {
                break;
            }
        }

        // Set debt for next step
        if self.gc_state == GcState::Pause {
            self.set_pause();
        } else {
            // Set negative debt to allow allocation before next GC step
            self.set_debt(-stepsize);
        }
    }

    /// Single GC step (like singlestep in Lua 5.5)
    fn single_step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool, fast: bool) -> StepResult {
        if self.gc_stopem {
            return StepResult::Work(0); // Emergency stop
        }

        self.gc_stopem = true; // Prevent reentrancy

        let result = match self.gc_state {
            GcState::Pause => {
                self.restart_collection(roots, pool);
                self.gc_state = GcState::Propagate;
                StepResult::Work(1)
            }
            GcState::Propagate => {
                if fast || self.gray.is_empty() {
                    self.gc_state = GcState::EnterAtomic;
                    StepResult::Work(1)
                } else {
                    let work = self.propagate_mark(pool);
                    StepResult::Work(work)
                }
            }
            GcState::EnterAtomic => {
                self.atomic(roots, pool);
                self.enter_sweep(pool);
                StepResult::Atomic
            }
            GcState::SwpAllGc => {
                let complete = self.sweep_step(pool, fast);
                if complete {
                    self.gc_state = GcState::SwpFinObj;
                }
                StepResult::Work(100) // GCSWEEPMAX equivalent
            }
            GcState::SwpFinObj => {
                let complete = self.sweep_step(pool, fast);
                if complete {
                    self.gc_state = GcState::SwpToBeFnz;
                }
                StepResult::Work(100)
            }
            GcState::SwpToBeFnz => {
                let complete = self.sweep_step(pool, fast);
                if complete {
                    self.gc_state = GcState::SwpEnd;
                }
                StepResult::Work(100)
            }
            GcState::SwpEnd => {
                self.gc_state = GcState::CallFin;
                StepResult::Work(100)
            }
            GcState::CallFin => {
                // In Lua 5.5, this would call finalizers if tobefnz is not empty
                // For now, we immediately transition to Pause
                // TODO: Implement proper finalizer calling
                self.gc_state = GcState::Pause;
                StepResult::Pause
            }
            GcState::Atomic => {
                // Should not reach here directly
                StepResult::Work(0)
            }
        };

        self.gc_stopem = false;
        result
    }

    /// Restart collection (like restartcollection in Lua 5.5)
    fn restart_collection(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.stats.collection_count += 1;
        self.gray.clear();
        self.grayagain.clear();
        self.gc_marked = 0;

        // Mark all objects as white
        self.make_all_white(pool);

        // Mark roots
        for value in roots {
            self.mark_value(value, pool);
        }
    }

    /// Make all objects white (prepare for new cycle)
    fn make_all_white(&mut self, pool: &mut ObjectPool) {
        let white = self.current_white;

        for (_id, table) in pool.tables.iter_mut() {
            if !table.header.is_fixed() {
                table.header.make_white(white);
            }
        }
        for (_id, func) in pool.functions.iter_mut() {
            if !func.header.is_fixed() {
                func.header.make_white(white);
            }
        }
        for (_id, upval) in pool.upvalues.iter_mut() {
            if !upval.header.is_fixed() {
                upval.header.make_white(white);
            }
        }
        for (_id, string) in pool.iter_strings_mut() {
            if !string.header.is_fixed() {
                string.header.make_white(white);
            }
        }
    }

    /// Mark a value (add to gray list if collectable)
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
            LuaValueKind::String => {
                if let Some(id) = value.as_string_id() {
                    if let Some(s) = pool.get_string_gc_mut(id) {
                        // Strings are leaves - mark black directly (but only if white)
                        if s.header.is_white() {
                            s.header.make_black();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Mark all constants in a chunk and its nested chunks (like Lua 5.5's traverseproto)
    fn mark_chunk_constants(&mut self, chunk: &Chunk, pool: &mut ObjectPool) {
        // Mark all constants in this chunk
        for constant in &chunk.constants {
            self.mark_value(constant, pool);
        }
        
        // Recursively mark constants in child protos (nested functions)
        for child_chunk in &chunk.child_protos {
            self.mark_chunk_constants(child_chunk, pool);
        }
    }

    fn mark_table_id(&mut self, table_id: TableId, pool: &mut ObjectPool) {
        if let Some(t) = pool.tables.get_mut(table_id.0) {
            if t.header.is_white() {
                t.header.make_gray();
                self.gray.push(GcId::TableId(table_id));
            }
        }
    }

    /// Propagate mark for one gray object (like propagatemark in Lua 5.5)
    fn propagate_mark(&mut self, pool: &mut ObjectPool) -> isize {
        if let Some(gc_id) = self.gray.pop() {
            let _ = self.mark_one(gc_id, pool);
            // Note: work returned by mark_one is the traversal work (number of objects/fields)
            // But gc_marked should track the SIZE of marked objects (like Lua's GCmarked)
            // We use the estimated size as the work cost, consistent with Lua's behavior (step size in KB)
            let size = self.estimate_object_size(gc_id, pool);
            self.gc_marked += size;
            size
        } else {
            0
        }
    }

    /// Estimate object size for GC accounting (like objsize in Lua)
    fn estimate_object_size(&self, gc_id: GcId, pool: &ObjectPool) -> isize {
        match gc_id {
            GcId::TableId(_) => 512,      // Reasonable average for test tables
            GcId::FunctionId(_) => 256,   // Base + some upvalues
            GcId::UpvalueId(_) => 64,     // Fixed size
            GcId::StringId(id) => {
                if let Some(s) = pool.get_string(id) {
                    32 + s.len() as isize
                } else {
                    64
                }
            }
            GcId::BinaryId(id) => {
                if let Some(b) = pool.binaries.get(id.0) {
                    32 + b.data.len() as isize
                } else {
                    64
                }
            }
            GcId::UserdataId(_) => 128,
            GcId::ThreadId(_) => 2048,    // Thread + Stack
        }
    }

    /// Mark one object and traverse its references
    fn mark_one(&mut self, gc_id: GcId, pool: &mut ObjectPool) -> isize {
        match gc_id {
            GcId::TableId(id) => {
                // First collect entries and metatable to mark
                let (entries, metatable) = if let Some(table) = pool.tables.get_mut(id.0) {
                    table.header.make_black();
                    let entries: Vec<_> = table.data.iter_all();
                    let metatable = table.data.get_metatable();
                    (entries, metatable)
                } else {
                    return 0;
                };

                // Then mark them (this may mutably borrow pool again)
                for (k, v) in &entries {
                    self.mark_value(k, pool);
                    self.mark_value(v, pool);
                }
                if let Some(mt) = &metatable {
                    self.mark_table_id(*mt, pool);
                }
                return 1 + entries.len() as isize;
            }
            GcId::FunctionId(id) => {
                // First mark the function black and get references to data we need
                let (upvalues, chunk) = if let Some(func) = pool.functions.get_mut(id.0) {
                    func.header.make_black();
                    let upvalues = func.data.upvalues();
                    let chunk = func.data.chunk().map(|c| c.clone()); // Clone Rc<Chunk>
                    (upvalues, chunk)
                } else {
                    return 0;
                };

                // Mark upvalues
                for upval_id in &upvalues {
                    if let Some(uv) = pool.upvalues.get_mut(upval_id.0) {
                        if uv.header.is_white() {
                            uv.header.make_gray();
                            self.gray.push(GcId::UpvalueId(*upval_id));
                        }
                    }
                }

                // Mark all constants in the chunk and nested chunks (like Lua 5.5's traverseproto)
                if let Some(chunk) = chunk {
                    self.mark_chunk_constants(&chunk, pool);
                    return 1 + upvalues.len() as isize + chunk.constants.len() as isize + chunk.child_protos.len() as isize;
                } else {
                    return 1 + upvalues.len() as isize;
                }
            }
            GcId::UpvalueId(id) => {
                // First get the closed value
                let closed_value = if let Some(upval) = pool.upvalues.get_mut(id.0) {
                    upval.header.make_black();
                    if !upval.data.is_open() {
                        Some(upval.data.get_closed_value().unwrap())
                    } else {
                        None
                    }
                } else {
                    return 0;
                };

                // Then mark it
                if let Some(val) = closed_value {
                    self.mark_value(&val, pool);
                }
                return 1;
            }
            GcId::StringId(id) => {
                if let Some(s) = pool.get_string_gc_mut(id) {
                    s.header.make_black();
                }
                return 1;
            }
            GcId::BinaryId(id) => {
                if let Some(b) = pool.binaries.get_mut(id.0) {
                    b.header.make_black();
                }
                return 1;
            }
            GcId::UserdataId(id) => {
                // Userdata: mark the userdata itself and its metatable if any
                let metatable = if let Some(ud) = pool.userdata.get_mut(id.0) {
                    ud.header.make_black();
                    ud.data.get_metatable() // Get metatable via public method
                } else {
                    return 0;
                };

                // Mark metatable if exists (it's a LuaValue, could be table)
                if let Some(mt_id) = metatable.as_table_id() {
                    self.mark_table_id(mt_id, pool);
                }
                return 1;
            }
            GcId::ThreadId(id) => {
                // Thread: mark all stack values and open upvalues
                let (stack_values, open_upvalues) = if let Some(thread) = pool.threads.get_mut(id.0)
                {
                    thread.header.make_black();
                    // Collect stack values up to stack_top
                    let state = &thread.data;
                    let stack_top = state.stack_top;
                    let stack_values: Vec<LuaValue> =
                        state.stack.iter().take(stack_top).copied().collect();

                    // Collect open upvalues using public getter
                    let open_upvalues: Vec<UpvalueId> = state.get_open_upvalues().to_vec();

                    (stack_values, open_upvalues)
                } else {
                    return 0;
                };

                // Mark all stack values
                for value in &stack_values {
                    self.mark_value(value, pool);
                }

                // Mark all open upvalues
                for upval_id in &open_upvalues {
                    if let Some(uv) = pool.upvalues.get_mut(upval_id.0) {
                        if uv.header.is_white() {
                            uv.header.make_gray();
                            self.gray.push(GcId::UpvalueId(*upval_id));
                        }
                    }
                }

                return 1 + stack_values.len() as isize;
            }
        }
    }

    /// Atomic phase (like atomic in Lua 5.5)
    fn atomic(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.gc_state = GcState::Atomic;

        // Mark roots again (they may have changed)
        for value in roots {
            self.mark_value(value, pool);
        }

        // Propagate all marks (empty the gray list)
        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }

        // Process grayagain list (objects that were blackened but then mutated)
        // Moving them to gray list effectively
        let grayagain = std::mem::take(&mut self.grayagain);
        for gc_id in grayagain {
            self.mark_one(gc_id, pool);
            
            // DRAIN GRAY IMMEDIATELY after each mark_one? 
            // Or just drain after all? Lua drains after all.
        }
        
        // Propagate again to handle anything pushed by grayagain processing
        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }

        // Flip white color for next cycle
        self.current_white ^= 1;
    }

    /// Enter sweep phase (like entersweep in Lua 5.5)
    pub fn enter_sweep(&mut self, _pool: &mut ObjectPool) {
        self.gc_state = GcState::SwpAllGc;
    }

    /// Sweep step - collect dead objects (like sweepstep in Lua 5.5)
    /// Returns true if sweep is complete (no more objects to sweep)
    fn sweep_step(&mut self, pool: &mut ObjectPool, fast: bool) -> bool {
        let max_sweep = if fast { usize::MAX } else { 100 }; // GCSWEEPMAX
        let mut swept = 0;

        // Get the "other white" - objects from the previous GC cycle that weren't marked
        let other_white = 1 - self.current_white;

        // Sweep tables
        let dead_tables: Vec<_> = pool
            .tables
            .iter()
            .filter(|(_, t)| !t.header.is_fixed() && t.header.is_dead(other_white))
            .map(|(id, _)| id)
            .take(max_sweep)
            .collect();

        for id in dead_tables {
            pool.tables.free(id);
            self.record_deallocation(256);
            self.stats.objects_collected += 1;
            swept += 1;
        }

        // Sweep functions
        if swept < max_sweep {
            let dead_funcs: Vec<_> = pool
                .functions
                .iter()
                .filter(|(_, f)| !f.header.is_fixed() && f.header.is_dead(other_white))
                .map(|(id, _)| id)
                .take(max_sweep - swept)
                .collect();

            for id in dead_funcs {
                pool.functions.free(id);
                self.record_deallocation(128);
                self.stats.objects_collected += 1;
                swept += 1;
            }
        }

        // Sweep upvalues
        if swept < max_sweep {
            let dead_upvals: Vec<_> = pool
                .upvalues
                .iter()
                .filter(|(_, u)| !u.header.is_fixed() && u.header.is_dead(other_white))
                .map(|(id, _)| id)
                .take(max_sweep - swept)
                .collect();

            for id in dead_upvals {
                pool.upvalues.free(id);
                self.record_deallocation(64);
                self.stats.objects_collected += 1;
                swept += 1;
            }
        }

        // Sweep strings
        if swept < max_sweep {
            let dead_strings: Vec<_> = pool
                .iter_strings()
                .filter(|(_, s)| !s.header.is_fixed() && s.header.is_dead(other_white))
                .map(|(id, _)| id)
                .take(max_sweep - swept)
                .collect();

            for id in dead_strings {
                pool.remove_string(StringId(id));
                self.record_deallocation(64);
                self.stats.objects_collected += 1;
                swept += 1;
            }
        }

        // Sweep userdata
        if swept < max_sweep {
            let dead_userdata: Vec<_> = pool
                .userdata
                .iter()
                .filter(|(_, u)| !u.header.is_fixed() && u.header.is_dead(other_white))
                .map(|(id, _)| id)
                .take(max_sweep - swept)
                .collect();

            for id in dead_userdata {
                pool.userdata.free(id);
                self.record_deallocation(128); // Approximate size
                self.stats.objects_collected += 1;
                swept += 1;
            }
        }

        // Sweep threads
        if swept < max_sweep {
            let dead_threads: Vec<_> = pool
                .threads
                .iter()
                .filter(|(_, t)| !t.header.is_fixed() && t.header.is_dead(other_white))
                .map(|(id, _)| id)
                .take(max_sweep - swept)
                .collect();

            for id in dead_threads {
                pool.threads.free(id);
                self.record_deallocation(512); // Approximate size (threads are big)
                self.stats.objects_collected += 1;
                swept += 1;
            }
        }

        // Return true if we didn't find enough objects to sweep (sweep complete)
        swept < max_sweep
    }

    /// Set pause (like setpause in Lua 5.5)
    pub fn set_pause(&mut self) {
        let threshold = self.apply_param(PAUSE, self.gc_marked);
        let debt = threshold - self.total_bytes;
        // Don't force debt to be non-negative! Negative debt means we have budget before next GC
        self.set_debt(debt);
    }
    /// Check if we need to keep invariant (like keepinvariant in Lua 5.5)
    /// During marking phase, the invariant must be kept
    pub fn keep_invariant(&self) -> bool {
        matches!(
            self.gc_state,
            GcState::Propagate | GcState::EnterAtomic | GcState::Atomic
        )
    }

    /// Run GC until reaching a specific state (like luaC_runtilstate in Lua 5.5)
    pub fn run_until_state(
        &mut self,
        target_state: GcState,
        roots: &[LuaValue],
        pool: &mut ObjectPool,
    ) {
        const MAX_ITERATIONS: usize = 1000;
        let mut iterations = 0;

        // Already at target state? Done.
        if self.gc_state == target_state {
            return;
        }

        // Special case: If we're trying to reach CallFin from Pause,
        // and we don't have finalizer support yet, CallFin will immediately
        // transition to Pause. We need to detect when we pass through CallFin.
        let mut just_left_callfin = false;

        loop {
            let prev_state = self.gc_state;
            self.single_step(roots, pool, true);
            let new_state = self.gc_state;
            iterations += 1;

            // Track if we just transitioned FROM CallFin to Pause
            if prev_state == GcState::CallFin && new_state == GcState::Pause {
                just_left_callfin = true;
            }

            // Check if we reached the target state
            if new_state == target_state {
                break;
            }

            // Special case: If target is CallFin and we just left it
            if target_state == GcState::CallFin && just_left_callfin {
                break;
            }

            if iterations >= MAX_ITERATIONS {
                // Safety check - should never happen in normal operation
                break;
            }
        }
    }

    /// Full generation collection (like fullgen in Lua 5.5)
    pub fn full_generation(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // For generational mode, do a major collection
        // Similar to fullinc in Lua 5.5

        // Restart collection
        self.restart_collection(roots, pool);

        // Mark all roots
        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }

        // Sweep using run_until_state
        self.enter_sweep(pool);
        self.run_until_state(GcState::CallFin, roots, pool);
        self.run_until_state(GcState::Pause, roots, pool);

        self.set_pause();
    }

    /// Set minor debt for generational mode
    fn set_minor_debt(&mut self) {
        // Use gc_marked as base if gc_majorminor is 0 (not yet set)
        let base = if self.gc_majorminor > 0 {
            self.gc_majorminor
        } else {
            self.gc_marked.max(1024 * 1024) // Reasonable default: 1MB
        };
        let debt = self.apply_param(MINORMUL, base);
        self.set_debt(-debt); // Negative = credit
    }

    /// Young collection for generational mode (placeholder)
    fn young_collection(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.stats.minor_collections += 1;

        // For now, just do a simple mark-sweep of young objects
        self.restart_collection(roots, pool);

        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }

        self.sweep_step(pool, true);

        self.gc_state = GcState::Pause;
    }

    // ============ GC Write Barriers (from lgc.c) ============

    /// Forward barrier (luaC_barrier_)
    /// Called when a black object 'o' is modified to point to white object 'v'
    /// This maintains the invariant: black objects cannot point to white objects
    ///
    /// From Lua 5.5:
    /// - If keepinvariant: mark 'v' immediately (restores invariant)
    /// - In generational mode: if 'o' is old, make 'v' OLD0 (generational invariant)
    /// - In sweep phase (incremental): make 'o' white to avoid repeated barriers
    pub fn barrier(&mut self, o_id: GcId, v_id: GcId, pool: &mut ObjectPool) {
        // Check if 'o' is black and 'v' is white
        let (o_is_black, o_is_old, v_is_white) = match (o_id, v_id) {
            (GcId::UpvalueId(oid), GcId::TableId(vid)) => {
                let o_black = pool
                    .upvalues
                    .get(oid.0)
                    .map(|o| o.header.is_black())
                    .unwrap_or(false);
                let o_old = pool
                    .upvalues
                    .get(oid.0)
                    .map(|o| o.header.is_old())
                    .unwrap_or(false);
                let v_white = pool
                    .tables
                    .get(vid.0)
                    .map(|v| v.header.is_white())
                    .unwrap_or(false);
                (o_black, o_old, v_white)
            }
            (GcId::UpvalueId(oid), GcId::FunctionId(vid)) => {
                let o_black = pool
                    .upvalues
                    .get(oid.0)
                    .map(|o| o.header.is_black())
                    .unwrap_or(false);
                let o_old = pool
                    .upvalues
                    .get(oid.0)
                    .map(|o| o.header.is_old())
                    .unwrap_or(false);
                let v_white = pool
                    .functions
                    .get(vid.0)
                    .map(|v| v.header.is_white())
                    .unwrap_or(false);
                (o_black, o_old, v_white)
            }
            (GcId::UpvalueId(oid), GcId::StringId(vid)) => {
                let o_black = pool
                    .upvalues
                    .get(oid.0)
                    .map(|o| o.header.is_black())
                    .unwrap_or(false);
                let o_old = pool
                    .upvalues
                    .get(oid.0)
                    .map(|o| o.header.is_old())
                    .unwrap_or(false);
                let v_white = pool
                    .get_string_gc_mut(vid)
                    .map(|v| v.header.is_white())
                    .unwrap_or(false);
                (o_black, o_old, v_white)
            }
            (GcId::TableId(oid), GcId::TableId(vid)) => {
                let o_black = pool
                    .tables
                    .get(oid.0)
                    .map(|o| o.header.is_black())
                    .unwrap_or(false);
                let o_old = pool
                    .tables
                    .get(oid.0)
                    .map(|o| o.header.is_old())
                    .unwrap_or(false);
                let v_white = pool
                    .tables
                    .get(vid.0)
                    .map(|v| v.header.is_white())
                    .unwrap_or(false);
                (o_black, o_old, v_white)
            }
            _ => return, // Unsupported combination
        };

        if !o_is_black || !v_is_white {
            return; // No barrier needed
        }

        // Must keep invariant during mark phase
        if self.gc_state.keep_invariant() {
            // Mark 'v' immediately to restore invariant
            self.mark_object(v_id, pool);

            // Generational invariant: if 'o' is old, make 'v' OLD0
            if o_is_old {
                match v_id {
                    GcId::TableId(id) => {
                        if let Some(t) = pool.tables.get_mut(id.0) {
                            t.header.make_old0();
                        }
                    }
                    GcId::FunctionId(id) => {
                        if let Some(f) = pool.functions.get_mut(id.0) {
                            f.header.make_old0();
                        }
                    }
                    _ => {}
                }
            }
        } else if self.gc_state.is_sweep_phase() {
            // In incremental sweep: make 'o' white to avoid repeated barriers
            if self.gc_kind != GcKind::GenMinor {
                match o_id {
                    GcId::UpvalueId(id) => {
                        if let Some(o) = pool.upvalues.get_mut(id.0) {
                            o.header.make_white(self.current_white);
                        }
                    }
                    GcId::TableId(id) => {
                        if let Some(o) = pool.tables.get_mut(id.0) {
                            o.header.make_white(self.current_white);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Backward barrier (luaC_barrierback_)
    /// Called when a black object 'o' is modified to point to white object
    /// Instead of marking the white object, we mark 'o' as gray again
    /// Used for tables and other objects that may have many modifications
    ///
    /// From Lua 5.5:
    /// - Link 'o' into grayagain list for re-traversal
    /// - If 'o' is old (generational): set age to TOUCHED1
    /// - If already TOUCHED2: just make it gray (will become TOUCHED1)
    pub fn barrier_back(&mut self, o_id: GcId, pool: &mut ObjectPool) {
        let (is_black, age) = match o_id {
            GcId::TableId(id) => pool
                .tables
                .get(id.0)
                .map(|o| (o.header.is_black(), o.header.age()))
                .unwrap_or((false, 0)),
            GcId::UpvalueId(id) => pool
                .upvalues
                .get(id.0)
                .map(|o| (o.header.is_black(), o.header.age()))
                .unwrap_or((false, 0)),
            _ => return,
        };

        if !is_black {
            return; // Only affects black objects
        }

        // In generational mode: check age constraints
        if self.gc_kind == GcKind::GenMinor {
            if age < G_OLD0 {
                return; // Young objects don't need backward barrier in minor mode
            }
            if age == G_TOUCHED1 {
                return; // Already in grayagain list
            }
        }

        // If TOUCHED2: just make gray (will become TOUCHED1 at end of cycle)
        if age == G_TOUCHED2 {
            match o_id {
                GcId::TableId(id) => {
                    if let Some(o) = pool.tables.get_mut(id.0) {
                        o.header.make_gray();
                    }
                }
                GcId::UpvalueId(id) => {
                    if let Some(o) = pool.upvalues.get_mut(id.0) {
                        o.header.make_gray();
                    }
                }
                _ => {}
            }
        } else {
            // Link into grayagain and make gray
            if !self.grayagain.contains(&o_id) {
                self.grayagain.push(o_id);
            }

            match o_id {
                GcId::TableId(id) => {
                    if let Some(o) = pool.tables.get_mut(id.0) {
                        o.header.make_gray();
                    }
                }
                GcId::UpvalueId(id) => {
                    if let Some(o) = pool.upvalues.get_mut(id.0) {
                        o.header.make_gray();
                    }
                }
                _ => {}
            }
        }

        // If old in generational mode: mark as TOUCHED1
        if age >= G_OLD0 {
            match o_id {
                GcId::TableId(id) => {
                    if let Some(o) = pool.tables.get_mut(id.0) {
                        o.header.make_touched1();
                    }
                }
                GcId::UpvalueId(id) => {
                    if let Some(o) = pool.upvalues.get_mut(id.0) {
                        o.header.make_touched1();
                    }
                }
                _ => {}
            }
        }
    }

    /// Mark an object (helper for barrier)
    fn mark_object(&mut self, gc_id: GcId, pool: &mut ObjectPool) {
        match gc_id {
            GcId::TableId(id) => {
                if let Some(t) = pool.tables.get_mut(id.0) {
                    if t.header.is_white() {
                        t.header.make_gray();
                        self.gray.push(gc_id);
                    }
                }
            }
            GcId::FunctionId(id) => {
                if let Some(f) = pool.functions.get_mut(id.0) {
                    if f.header.is_white() {
                        f.header.make_gray();
                        self.gray.push(gc_id);
                    }
                }
            }
            GcId::StringId(id) => {
                if let Some(s) = pool.get_string_gc_mut(id) {
                    s.header.make_black(); // Strings are leaves
                }
            }
            GcId::BinaryId(id) => {
                if let Some(b) = pool.binaries.get_mut(id.0) {
                    b.header.make_black(); // Binary data is a leaf
                }
            }
            _ => {}
        }
    }
}

/// Result of a GC step
enum StepResult {
    Work(isize), // Amount of work done
    Pause,       // Reached pause state
    Atomic,      // Completed atomic phase
    #[allow(unused)]
    MinorMode, // Returned to minor mode
}

impl Default for GC {
    fn default() -> Self {
        Self::new()
    }
}
