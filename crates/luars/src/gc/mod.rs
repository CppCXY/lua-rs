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

mod gc_id;
mod gc_object;
mod object_pool;
mod string_interner;

use crate::lua_value::{Chunk, LuaValue, LuaValueKind};
pub use gc_id::*;
pub use gc_object::*;
pub use object_pool::*;

/// Actions that GC needs VM to perform after a GC step
/// This allows GC to mark objects for finalization or weak table cleanup
/// without needing access to LuaState during the GC phase
#[derive(Default, Debug)]
pub struct GcActions {
    /// Objects that need their __gc finalizer called
    pub to_finalize: Vec<GcId>,
    /// Weak tables that need key/value cleanup
    pub weak_tables: Vec<gc_id::TableId>,
}

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

    // === Pending actions (for finalizers and weak tables) ===
    /// Accumulated actions that need VM to process
    /// This is filled during GC steps and retrieved by VM later
    pending_actions: GcActions,

    // === GC parameters (from gcparams[LUA_GCPN]) ===
    pub gc_params: [i32; GCPARAM_COUNT],

    // === Gray lists (for marking) ===
    /// Regular gray objects waiting to be visited
    pub gray: Vec<GcId>,

    /// Objects to be revisited at atomic phase
    pub grayagain: Vec<GcId>,

    // === Sweep state ===
    /// Current position in sweep (like Lua 5.5's sweepgc pointer)
    /// This ensures we don't re-scan the same objects
    sweep_index: usize,

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
            pending_actions: GcActions::default(),
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
            sweep_index: 0,
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
    ///
    /// **CRITICAL**: Objects are created WHITE by ObjectPool.create_*() with current_white.
    /// This function ONLY tracks memory accounting - it does NOT modify object colors.
    /// The tri-color invariant is maintained by write barriers, not by track_object.
    ///
    /// Port of lgc.c: luaC_newobj creates objects as WHITE, then links to allgc list.
    /// Barriers will mark them BLACK/GRAY if needed when stored into reachable objects.
    #[inline]
    pub fn track_object(&mut self, gc_id: GcId, pool: &mut ObjectPool) {
        // Objects are already created as WHITE by ObjectPool.create_*()
        // We do NOT modify color here - this is ONLY for memory accounting
        //
        // The Lua 5.5 way:
        // 1. luaC_newobj() creates object as WHITE (current white)
        // 2. Object is linked to allgc list
        // 3. If stored to stack/table/etc, it's immediately reachable from roots
        // 4. Next GC mark phase will mark it from roots
        // 5. If NOT stored anywhere, it becomes garbage in current cycle (correct!)
        //
        // Our previous code INCORRECTLY marked objects as GRAY/BLACK here,
        // which violates Lua's design and prevents collection of unreachable objects.

        // Use the size stored in GcObject (calculated at creation time)
        // This ensures perfect consistency with sweep deallocation
        let size = if let Some(obj) = pool.gc_pool.get(gc_id.index()) {
            obj.size() as usize
        } else {
            0
        };

        let size_signed = size as isize;
        self.total_bytes += size_signed;
        self.gc_debt += size_signed;
        self.stats.bytes_allocated += size;
    }

    /// Check if GC should run (debt > 0)
    #[inline]
    pub fn should_collect(&self) -> bool {
        self.gc_debt > 0
    }

    /// Set GC debt (like luaE_setdebt in Lua)
    /// gc_debt > 0: need GC
    /// gc_debt < 0: have budget before next GC
    pub fn set_debt(&mut self, debt: isize) {
        // Simply set the debt - don't modify total_bytes!
        // total_bytes tracks actual memory, gc_debt tracks GC scheduling
        self.gc_debt = debt;
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

    // ============ Pending Actions Management ============

    /// Get accumulated GC actions that need VM processing
    /// This retrieves and clears the pending actions
    pub fn take_pending_actions(&mut self) -> GcActions {
        std::mem::take(&mut self.pending_actions)
    }

    /// Check if there are pending actions waiting for VM
    pub fn has_pending_actions(&self) -> bool {
        !self.pending_actions.to_finalize.is_empty() 
            || !self.pending_actions.weak_tables.is_empty()
    }

    /// Enter finalizer execution mode - temporarily stop GC to prevent
    /// objects from being collected while their finalizers are running
    pub fn enter_finalizer_mode(&mut self) {
        self.gc_stopem = true;
    }

    /// Exit finalizer execution mode - resume normal GC operation
    pub fn exit_finalizer_mode(&mut self) {
        self.gc_stopem = false;
    }

    /// Check if a GcId represents a dead object (will be collected)
    /// Used by weak table cleanup to identify dead keys/values
    pub fn is_object_dead(&self, gc_id: GcId, pool: &ObjectPool) -> bool {
        if let Some(obj) = pool.gc_pool.get(gc_id.index()) {
            // Fixed objects are never dead
            if obj.header.is_fixed() {
                return false;
            }
            // Calculate other_white (the white that will be collected)
            let other_white = gc_object::GcHeader::otherwhite(self.current_white);
            // Object is dead if it's marked with other_white
            obj.header.is_dead(other_white)
        } else {
            // Object doesn't exist = dead
            true
        }
    }

    // ============ Core GC Implementation ============

    /// Main GC step function (like luaC_step in Lua 5.5)
    /// Actions are accumulated in pending_actions, retrieve with take_pending_actions()
    pub fn step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.step_internal(roots, pool, false)
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
                self.inc_step(roots, pool)
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

        // Use i128 to prevent overflow when debt and stepmul are both very large
        let mut work2do = ((effective_debt as i128 * stepmul as i128) / 100) as isize;

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
                    // Only one gc_pool, skip finobj and tobefnz phases
                    self.gc_state = GcState::SwpEnd;
                }
                StepResult::Work(100) // GCSWEEPMAX equivalent
            }
            GcState::SwpFinObj => {
                // Skip: we don't have separate finobj list
                self.gc_state = GcState::SwpToBeFnz;
                StepResult::Work(1)
            }
            GcState::SwpToBeFnz => {
                // Skip: we don't have separate tobefnz list
                self.gc_state = GcState::SwpEnd;
                StepResult::Work(1)
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

        // Flip current_white first (atomic phase already flipped it, but double-check)
        // Objects from last cycle are now "other white" and will be collected if not marked
        // NO need to make_all_white - objects are already white from last cycle!

        // Mark roots
        for value in roots.iter() {
            self.mark_value(value, pool);
        }
    }

    /// Mark a value (add to gray list if collectable)
    /// Like Lua 5.5's markvalue: "{ if (valiswhite(o)) reallymarkobject(g,gcvalue(o)); }"
    /// Fixed objects are GRAY, so they naturally skip the white check
    fn mark_value(&mut self, value: &LuaValue, pool: &mut ObjectPool) {
        match value.kind() {
            LuaValueKind::Table => {
                if let Some(id) = value.as_table_id() {
                    if let Some(t) = pool.get_mut(id.into()) {
                        // Only mark white objects (fixed objects are gray, so they're skipped)
                        if t.header.is_white() {
                            t.header.make_gray();
                            self.gray.push(GcId::TableId(id));
                        }
                    }
                }
            }
            LuaValueKind::Function => {
                if let Some(id) = value.as_function_id() {
                    if let Some(f) = pool.get_mut(id.into()) {
                        // Only mark white objects (fixed objects are gray, so they're skipped)
                        if f.header.is_white() {
                            f.header.make_gray();
                            self.gray.push(GcId::FunctionId(id));
                        }
                    }
                }
            }
            LuaValueKind::String => {
                if let Some(id) = value.as_string_id() {
                    if let Some(s) = pool.get_mut(id.into()) {
                        // Fixed strings (metamethod names) are gray forever, skip them
                        // Only mark white strings (fixed are gray, so naturally skipped)
                        if s.header.is_white() {
                            s.header.make_black(); // Strings are leaves
                        }
                    }
                }
            }
            LuaValueKind::Binary => {
                if let Some(id) = value.as_binary_id() {
                    if let Some(b) = pool.get_mut(id.into()) {
                        // Binaries are leaves - mark black directly (but only if white)
                        if b.header.is_white() {
                            b.header.make_black();
                        }
                    }
                }
            }
            LuaValueKind::Userdata => {
                if let Some(id) = value.as_userdata_id() {
                    if let Some(u) = pool.get_mut(id.into()) {
                        if u.header.is_white() {
                            u.header.make_gray();
                            self.gray.push(GcId::UserdataId(id));
                        }
                    }
                }
            }
            LuaValueKind::Thread => {
                if let Some(id) = value.as_thread_id() {
                    if let Some(t) = pool.get_mut(id.into()) {
                        if t.header.is_white() {
                            t.header.make_gray();
                            self.gray.push(GcId::ThreadId(id));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Mark all constants in a chunk and its nested chunks (like Lua 5.5's traverseproto)
    fn mark_chunk_constants(&mut self, chunk: &Chunk, pool: &mut ObjectPool, _func_id: FunctionId) {
        // Mark all constants in this chunk
        for constant in &chunk.constants {
            self.mark_value(constant, pool);
        }

        // Recursively mark constants in child protos (nested functions)
        for child_chunk in &chunk.child_protos {
            self.mark_chunk_constants(child_chunk, pool, _func_id);
        }
    }

    /// Propagate mark for one gray object (like propagatemark in Lua 5.5)
    fn propagate_mark(&mut self, pool: &mut ObjectPool) -> isize {
        if let Some(gc_id) = self.gray.pop() {
            let _ = self.mark_one(gc_id, pool);
            // Use the size stored in GcObject (same as track_object and sweep)
            // This ensures consistency across all GC operations
            let size = if let Some(obj) = pool.gc_pool.get(gc_id.index()) {
                obj.size() as isize
            } else {
                0
            };
            self.gc_marked += size;
            size
        } else {
            0
        }
    }

    /// Mark one object and traverse its references
    /// Like Lua 5.5's propagatemark: "nw2black(o);" then traverse
    /// Sets object to BLACK before traversing children
    fn mark_one(&mut self, gc_id: GcId, pool: &mut ObjectPool) -> isize {
        match gc_id {
            GcId::TableId(id) => {
                // Set to black first, then traverse (like Lua 5.5 propagatemark)
                let (entries, metatable) = if let Some(gc_table) = pool.get_mut(id.into()) {
                    gc_table.header.make_black();

                    let table = match gc_table.ptr.as_table_mut() {
                        Some(t) => t,
                        None => return 0,
                    };

                    let entries: Vec<_> = table.iter_all();
                    let metatable = table.get_metatable();

                    (entries, metatable)
                } else {
                    return 0;
                };

                // Then mark them (this may mutably borrow pool again)
                for (k, v) in &entries {
                    self.mark_value(k, pool);
                    self.mark_value(v, pool);
                }
                if let Some(mt_id) = metatable
                    && let Some(mt) = pool.get_table_value(mt_id)
                {
                    self.mark_value(&mt, pool);
                }
                return 1 + entries.len() as isize;
            }
            GcId::FunctionId(id) => {
                // Mark the function black and get references to data we need
                // (Fixed functions should never reach here - they stay gray forever)
                let (upvalues, chunk) = if let Some(gc_func) = pool.get_mut(id.into()) {
                    gc_func.header.make_black();
                    if let Some(func) = gc_func.ptr.as_function_mut() {
                        let upvalues = func.cached_upvalues().clone(); // Clone Vec<UpvalueId>
                        let chunk = func.chunk().map(|c| c.clone()); // Clone Rc<Chunk>
                        (upvalues, chunk)
                    } else {
                        // This shouldn't happen - FunctionId should always point to a Function
                        return 0;
                    }
                } else {
                    return 0;
                };

                // Mark upvalues
                for cache_up in &upvalues {
                    if let Some(uv) = pool.get_mut(cache_up.id.into()) {
                        if uv.header.is_white() {
                            uv.header.make_gray();
                            self.gray.push(GcId::UpvalueId(cache_up.id));
                        }
                    }
                }

                // Mark all constants in the chunk and nested chunks (like Lua 5.5's traverseproto)
                if let Some(chunk) = chunk {
                    self.mark_chunk_constants(&chunk, pool, id);
                    return 1
                        + upvalues.len() as isize
                        + chunk.constants.len() as isize
                        + chunk.child_protos.len() as isize;
                } else {
                    return 1 + upvalues.len() as isize;
                }
            }
            GcId::UpvalueId(id) => {
                // First get the closed value
                let closed_value = if let Some(gc_upval) = pool.get_mut(id.into()) {
                    gc_upval.header.make_black();
                    let upvalue = gc_upval.ptr.as_upvalue_mut().unwrap();
                    if !upvalue.is_open() {
                        Some(upvalue.get_closed_value().unwrap())
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
                if let Some(s) = pool.get_mut(id.into()) {
                    s.header.make_black();
                }
                return 1;
            }
            GcId::BinaryId(id) => {
                if let Some(b) = pool.get_mut(id.into()) {
                    b.header.make_black();
                }
                return 1;
            }
            GcId::UserdataId(id) => {
                // Userdata: mark the userdata itself and its metatable if any
                let metatable = if let Some(gc_ud) = pool.get_mut(id.into()) {
                    gc_ud.header.make_black();
                    let userdata = gc_ud.ptr.as_userdata_mut().unwrap();
                    userdata.get_metatable() // Get metatable via public method
                } else {
                    return 0;
                };

                // Mark metatable if exists (it's a LuaValue, could be table)
                self.mark_value(&metatable, pool);
                return 1;
            }
            GcId::ThreadId(id) => {
                // Thread: mark all stack values and open upvalues
                let (stack_values, open_upvalues) = if let Some(gc_thread) = pool.get_mut(id.into())
                {
                    gc_thread.header.make_black();
                    // Collect stack values up to stack_top
                    let state = gc_thread.ptr.as_thread_mut().unwrap();
                    let stack_top = state.stack_top;
                    let stack_values = state
                        .stack
                        .iter()
                        .take(stack_top)
                        .copied()
                        .collect::<Vec<_>>();

                    // Collect open upvalues using public getter
                    let open_upvalues = state.get_open_upvalues().to_vec();

                    (stack_values, open_upvalues)
                } else {
                    return 0;
                };

                // Mark all stack values
                for value in &stack_values {
                    self.mark_value(value, pool);
                }

                // Mark all open upvalues
                for upval_id in open_upvalues {
                    if let Some(gc_uv) = pool.get_mut(upval_id.into()) {
                        if gc_uv.header.is_white() {
                            gc_uv.header.make_gray();
                            self.gray.push(GcId::UpvalueId(upval_id));
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
        self.sweep_index = 0; // Reset sweep position

        let _old_white = self.current_white;
    }

    /// Sweep step - collect dead objects (like sweepstep in Lua 5.5)
    /// Returns true if sweep is complete (no more objects to sweep)
    ///
    /// Port of Lua 5.5's sweepstep: maintains sweep position (sweepgc pointer)
    /// to avoid re-scanning already-swept objects
    fn sweep_step(&mut self, pool: &mut ObjectPool, fast: bool) -> bool {
        let max_sweep = if fast { usize::MAX } else { 100 };
        let mut count = 0;
        let other_white = 1 - self.current_white;

        // Get current total slots (may grow during sweep if new objects are allocated)
        // In Lua 5.5, sweepgc iterates through linked list which naturally includes all objects
        // Here we must re-check capacity to handle objects allocated during sweep
        let current_total_slots = pool.gc_pool.capacity();

        let mut dead_ids = Vec::new();

        // Continue from where we left off (like Lua's sweepgc pointer)
        // Sweep through Vec slots directly (some may be None)
        while self.sweep_index < current_total_slots && count < max_sweep {
            let slot_index = self.sweep_index as u32;

            // Check if this slot has an object
            if let Some(obj) = pool.gc_pool.get(slot_index) {
                // Check if object is dead (other white and not fixed)
                if !obj.header.is_fixed() && obj.header.is_dead(other_white) {
                    // Convert slot_index to GcId using the object's type
                    let gc_id = obj.trans_to_gcid(slot_index);
                    dead_ids.push(gc_id);
                }
            }

            self.sweep_index += 1;
            count += 1; // Count every slot, not just objects, to avoid infinite loops
        }

        for gc_id in &dead_ids {
            let size = pool.remove(*gc_id);
            if size > 0 {
                self.total_bytes = self.total_bytes.saturating_sub(size as isize);
                self.stats.bytes_freed += size;
                self.stats.objects_collected += 1;
            }
        }

        // Return true if we've reached the end of all current slots
        // Note: new objects may be allocated after this sweep_step, so we check again next time
        self.sweep_index >= current_total_slots
    }

    pub fn set_pause(&mut self) {
        let threshold = self.apply_param(PAUSE, self.total_bytes);
        let debt = threshold - self.total_bytes;
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
        // Increase MAX_ITERATIONS to handle large object pools
        // With 100 objects per sweep step, we need more iterations for large heaps
        const MAX_ITERATIONS: usize = 100000;
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
        self.restart_collection(roots, pool);

        // Mark all roots
        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }

        // Flip white and sweep
        self.current_white ^= 1;
        self.enter_sweep(pool);
        self.run_until_state(GcState::CallFin, roots, pool);
        self.run_until_state(GcState::Pause, roots, pool);

        // set_pause uses total_bytes (actual memory after sweep) as base
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
    pub fn barrier(&mut self, o_id: GcId, v_id: GcId, pool: &mut ObjectPool) {
        // Check if 'o' is black and 'v' is white
        let (o_black, o_old) = if let Some(o) = pool.get(o_id) {
            (o.header.is_black(), o.header.is_old())
        } else {
            return;
        };

        if !o_black {
            return;
        }

        let v_white = if let Some(v) = pool.get(v_id) {
            v.header.is_white()
        } else {
            return;
        };

        if !v_white {
            return;
        }

        // Must keep invariant during mark phase
        if self.gc_state.keep_invariant() {
            // Mark 'v' immediately to restore invariant
            self.mark_object(v_id, pool);

            // Generational invariant: if 'o' is old, make 'v' OLD0
            if o_old {
                if let Some(v) = pool.get_mut(v_id) {
                    v.header.make_old0();
                }
            }
        } else if self.gc_state.is_sweep_phase() {
            // In incremental sweep: make 'o' white to avoid repeated barriers
            if self.gc_kind != GcKind::GenMinor {
                if let Some(o) = pool.get_mut(o_id) {
                    o.header.make_white(self.current_white);
                }
            }
        }
    }

    /// Backward barrier (luaC_barrierback_)
    /// Called when a black object 'o' is modified to point to white object
    /// Instead of marking the white object, we mark 'o' as gray again
    /// Used for tables and other objects that may have many modifications
    pub fn barrier_back(&mut self, o_id: GcId, pool: &mut ObjectPool) {
        let (is_black, age) = if let Some(o) = pool.get(o_id) {
            (o.header.is_black(), o.header.age())
        } else {
            return;
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
            if let Some(o) = pool.get_mut(o_id) {
                o.header.make_gray();
            }
        } else {
            // Link into grayagain and make gray
            if !self.grayagain.contains(&o_id) {
                self.grayagain.push(o_id);
            }

            if let Some(o) = pool.get_mut(o_id) {
                o.header.make_gray();
            }
        }

        // If old in generational mode: mark as TOUCHED1
        if age >= G_OLD0 {
            if let Some(o) = pool.get_mut(o_id) {
                o.header.make_touched1();
            }
        }
    }

    /// Mark an object (helper for barrier)
    fn mark_object(&mut self, gc_id: GcId, pool: &mut ObjectPool) {
        if let Some(obj) = pool.get_mut(gc_id) {
            // Only need to mark if it is white
            if obj.header.is_white() {
                match obj.ptr {
                    GcPtrObject::String(_) | GcPtrObject::Binary(_) => {
                        obj.header.make_black(); // Leaves become black immediately
                    }
                    _ => {
                        obj.header.make_gray(); // Others become gray
                        self.gray.push(gc_id);
                    }
                }
            }
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
