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
/// This allows GC to mark objects for finalization
/// Weak tables are now cleaned directly during GC atomic phase
#[derive(Default, Debug)]
pub struct GcActions {
    /// Objects that need their __gc finalizer called
    pub to_finalize: Vec<GcId>,
}

// GC Parameters (from lua.h)
pub const PAUSE: usize = 0; // Pause between GC cycles (default 200%)
pub const STEPMUL: usize = 1; // GC speed multiplier (default 200)
pub const STEPSIZE: usize = 2; // Step size in KB (default 13KB)
pub const MINORMUL: usize = 3; // Minor collection multiplier (default 20%)
pub const MINORMAJOR: usize = 4; // Shift from minor to major (default 100%)
pub const MAJORMINOR: usize = 5; // Shift from major to minor (default 100%)
pub const GCPARAM_COUNT: usize = 6;

// Default GC parameters (from Lua 5.5 lgc.h)
// MUST match Lua 5.5 exactly for debugging consistency
const DEFAULT_PAUSE: i32 = 200; // 200% (LUAI_GCPAUSE in lgc.h)
const DEFAULT_STEPMUL: i32 = 200; // 200% (LUAI_GCMUL in lgc.h)
// LUAI_GCSTEPSIZE = (200 * sizeof(Table))
// 在Rust中Table大小约为80字节，所以 200 * 80 = 16000字节
// 但参数存储的是"单位"，在applygcparam时会乘以参数
// 实际上STEPSIZE存储的是200，表示"200 * 某个基础单位"
// 看lgc.c: applygcparam(g, STEPSIZE, 100) 意思是 (STEPSIZE * 100) / 100 = STEPSIZE
// 然后setgcparam(g, STEPSIZE, LUAI_GCSTEPSIZE) 存储的就是字节数
// 所以应该存储字节数！
const DEFAULT_STEPSIZE: i32 = 16000; // ~200 * sizeof(Table) bytes
const DEFAULT_MINORMUL: i32 = 20; // 20%
const DEFAULT_MINORMAJOR: i32 = 100; // 100%
const DEFAULT_MAJORMINOR: i32 = 100; // 100%

/// Maximum l_mem value (like MAX_LMEM in Lua 5.5)
const MAX_LMEM: isize = isize::MAX;
/// Compute ceil(log2(x)) for GC parameter encoding
/// Port of luaO_ceillog2 from Lua 5.5 lobject.c
fn ceil_log2(x: u32) -> u8 {
    static LOG_2: [u8; 256] = [
        0,1,2,2,3,3,3,3,4,4,4,4,4,4,4,4,5,5,5,5,5,5,5,5,5,5,5,5,5,5,5,5,
        6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,6,
        7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,
        7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,
        8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,
        8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,
        8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,
        8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,
    ];
    let mut x = x.saturating_sub(1);
    let mut l: u32 = 0;
    while x >= 256 {
        l += 8;
        x >>= 8;
    }
    (l as u8) + LOG_2[x as usize]
}

/// Encode a percentage value 'p' as a floating-point byte (eeeexxxx).
/// Port of luaO_codeparam from Lua 5.5 lobject.c
/// 
/// The exponent is represented using excess-7. Mimicking IEEE 754, the
/// representation normalizes the number when possible, assuming an extra
/// 1 before the mantissa (xxxx) and adding one to the exponent (eeee)
/// to signal that. So, the real value is (1xxxx) * 2^(eeee - 7 - 1) if
/// eeee != 0, and (xxxx) * 2^-7 otherwise (subnormal numbers).
pub fn code_param(p: u32) -> u8 {
    // Overflow check: maximum representable value
    // (0x1F) << (0xF - 7 - 1) = 31 << 7 = 3968
    // 3968 * 100 = 396800
    if p >= ((0x1Fu64) << (0xF - 7 - 1)) as u32 * 100 {
        return 0xFF; // Return maximum value on overflow
    }
    
    // p' = (p * 128 + 99) / 100 (round up the division)
    let p_scaled = ((p as u64) * 128 + 99) / 100;
    
    if p_scaled < 0x10 {
        // Subnormal number: exponent bits are already zero
        p_scaled as u8
    } else {
        // p >= 0x10 implies ceil(log2(p + 1)) >= 5
        // Preserve 5 bits in 'p'
        let log = ceil_log2((p_scaled + 1) as u32).saturating_sub(5);
        let mantissa = ((p_scaled >> log) - 0x10) as u8;
        let exponent = ((log as u8) + 1) << 4;
        mantissa | exponent
    }
}

/// Decode a floating-point byte back to approximate percentage
/// Used for returning parameter values to Lua
/// This is the inverse of code_param: given the encoded byte, return approximate percentage
/// 
/// The key insight is: apply_param(p, 100) ≈ original_percentage
/// Because apply_param computes: x * percentage / 100
/// So apply_param(p, 100) = 100 * percentage / 100 = percentage
pub fn decode_param(p: u8) -> i32 {
    let m = (p & 0xF) as isize;
    let e = (p >> 4) as i32;
    
    // Compute what apply_param(p, 100) would return
    // This gives us the original percentage value
    let x: isize = 100;
    
    let (m_full, e_adj) = if e > 0 {
        (m + 0x10, e - 1 - 7)
    } else {
        (m, -7)
    };
    
    if e_adj >= 0 {
        let e_adj = e_adj as u32;
        ((x * m_full) << e_adj) as i32
    } else {
        let e_neg = (-e_adj) as u32;
        ((x * m_full) >> e_neg) as i32
    }
}

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
    /// GC debt (like Lua 5.5 GCdebt)
    /// When debt <= 0, a GC step should run (allocation decreases debt)
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
    // Stored as compressed floating-point bytes (like Lua 5.5)
    // Use code_param() to encode, apply_param() to apply
    pub gc_params: [u8; GCPARAM_COUNT],

    // === Gray lists (for marking) ===
    /// Regular gray objects waiting to be visited
    pub gray: Vec<GcId>,

    /// Objects to be revisited at atomic phase
    pub grayagain: Vec<GcId>,

    // === Weak table lists (Port of Lua 5.5) ===
    /// Weak value tables (only values are weak)
    pub weak: Vec<TableId>,

    /// Ephemeron tables (keys are weak, but key存活则value存活)
    pub ephemeron: Vec<TableId>,

    /// Fully weak tables (both keys and values are weak)
    pub allweak: Vec<TableId>,

    // === Sweep state ===
    /// Current position in sweep (like Lua 5.5's sweepgc pointer)
    /// This ensures we don't re-scan the same objects
    sweep_index: usize,
    /// Target position for sweep completion (set at start of sweep phase)
    /// This prevents chasing a growing capacity() during sweep
    sweep_target: usize,
    /// List of object IDs to sweep (collected at start of sweep phase)
    /// This avoids iterating through sparse Vec slots
    sweep_ids: Vec<GcId>,

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
                code_param(DEFAULT_PAUSE as u32),      // PAUSE = 0
                code_param(DEFAULT_STEPMUL as u32),    // STEPMUL = 1
                code_param(DEFAULT_STEPSIZE as u32),   // STEPSIZE = 2
                code_param(DEFAULT_MINORMUL as u32),   // MINORMUL = 3
                code_param(DEFAULT_MINORMAJOR as u32), // MINORMAJOR = 4
                code_param(DEFAULT_MAJORMINOR as u32), // MAJORMINOR = 5
            ],
            gray: Vec::with_capacity(128),
            grayagain: Vec::with_capacity(64),
            weak: Vec::new(),
            ephemeron: Vec::new(),
            allweak: Vec::new(),
            sweep_index: 0,
            sweep_target: 0,
            sweep_ids: Vec::new(),
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
        let stepsize = self.apply_param(STEPSIZE, 100);
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
    ///
    /// Lua 5.5内存分配：lmem.c中 g->GCdebt -= size (分配减少debt)
    /// 维护不变量：GCtotalbytes = 实际分配字节 + GCdebt
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
        
        // Lua 5.5 lmem.c luaM_malloc_:
        //   g->GCdebt -= cast(l_mem, size);
        // 只修改GCdebt，不修改GCtotalbytes！
        // 不变量：真实内存 = GCtotalbytes - GCdebt
        // 分配时：debt减少size，totalbytes不变，所以真实内存增加size
        self.gc_debt -= size_signed;  // 分配减少debt
        // total_bytes保持不变！只在set_debt时调整
        
        self.stats.bytes_allocated += size;
    }

    /// Check if GC should run (debt > 0)
    #[inline]
    /// Check if GC should run (like Lua 5.5: G(L)->GCdebt <= 0)
    pub fn should_collect(&self) -> bool {
        self.gc_debt <= 0
    }

    // /*
    // ** set GCdebt to a new value keeping the real number of allocated
    // ** objects (GCtotalobjs - GCdebt) invariant and avoiding overflows in
    // ** 'GCtotalobjs'.
    // */
    // void luaE_setdebt (global_State *g, l_mem debt) {
    //   l_mem tb = gettotalbytes(g);
    //   lua_assert(tb > 0);
    //   if (debt > MAX_LMEM - tb)
    //     debt = MAX_LMEM - tb;  /* will make GCtotalbytes == MAX_LMEM */
    //   g->GCtotalbytes = tb + debt;
    //   g->GCdebt = debt;
    // }
    pub fn set_debt(&mut self, mut debt: isize) {
        // Port of Lua 5.5's luaE_setdebt from lstate.c
        // Keep the real allocated bytes (total_bytes - gc_debt) invariant
        let real_bytes = self.total_bytes - self.gc_debt;
        
        
        // Avoid overflow in total_bytes
        if debt > MAX_LMEM - real_bytes {
            debt = MAX_LMEM - real_bytes;
        }
        
        // Maintain invariant: total_bytes = real_bytes + debt
        self.total_bytes = real_bytes + debt;
        self.gc_debt = debt;
    }

    /// Apply GC parameter (like luaO_applyparam in Lua 5.5)
    /// Parameters are stored as compressed floating-point bytes (eeeexxxx)
    /// 
    /// Port of luaO_applyparam from lobject.c:
    /// Computes 'p' times 'x', where 'p' is a floating-point byte.
    /// Returns MAX_LMEM on overflow to prevent extreme debt values.
    pub(crate) fn apply_param(&self, param_idx: usize, value: isize) -> isize {
        let p = self.gc_params[param_idx];
        let x = value;
        
        let m = (p & 0xF) as isize;  // mantissa
        let e = (p >> 4) as i32;     // exponent
        
        let (m_full, e_adj) = if e > 0 {
            // Normalized number: add implicit 1 to mantissa
            (m + 0x10, e - 1 - 7)
        } else {
            // Subnormal number
            (m, -7)
        };
        
        if e_adj >= 0 {
            let e_adj = e_adj as u32;
            // Check for overflow before computing
            let max_safe = (MAX_LMEM / 0x1F) >> e_adj;
            if x < max_safe {
                (x * m_full) << e_adj
            } else {
                // Real overflow - return maximum
                MAX_LMEM
            }
        } else {
            // Negative exponent
            let e_neg = (-e_adj) as u32;
            if x < MAX_LMEM / 0x1F {
                // Multiplication cannot overflow, multiply first for precision
                (x * m_full) >> e_neg
            } else if (x >> e_neg) < MAX_LMEM / 0x1F {
                // Cannot overflow after shift
                (x >> e_neg) * m_full
            } else {
                // Real overflow
                MAX_LMEM
            }
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

    /// Port of Lua 5.5's iscleared function from lgc.c
    /// Check if an object is cleared (should be removed from weak table)
    /// For strings: marks them black and returns false (strings are 'values', never weak)
    /// For other objects: returns true if white (will be collected)
    fn is_cleared(&mut self, gc_id: GcId, pool: &mut ObjectPool) -> bool {
        match gc_id {
            GcId::StringId(string_id) => {
                // Strings are 'values', so are never weak
                // Mark the string black directly (strings have nothing to visit)
                // Port of: markobject(g,o) -> reallymarkobject -> set2black for strings
                if let Some(gc_obj) = pool.gc_pool.get_mut(string_id.0) {
                    if gc_obj.header.is_white() {
                        let size = gc_obj.header.size;
                        gc_obj.header.make_black();
                        self.gc_marked += size as isize;
                    }
                }
                false
            }
            _ => {
                // For other objects, check if they're white (will be collected)
                if let Some(obj) = pool.gc_pool.get(gc_id.index()) {
                    obj.header.is_white()
                } else {
                    true
                }
            }
        }
    }

    /// Check if an object needs finalization (__gc metamethod)
    /// Only tables, userdata, and threads can have __gc
    fn needs_finalization(&self, gc_id: GcId, pool: &ObjectPool) -> bool {
        let result = match gc_id {
            GcId::TableId(table_id) => {
                // Check if table has __gc metamethod
                if let Some(table_value) = pool.get_table_value(table_id) {
                    if let Some(table) = table_value.as_table() {
                        if let Some(mt_id) = table.get_metatable() {
                            if let Some(mt) = pool.get_table_value(mt_id) {
                                if let Some(mt_table) = mt.as_table() {
                                    let gc_key = pool.tm_gc.clone();
                                    return mt_table.raw_get(&gc_key).is_some()
                                        && !mt_table.raw_get(&gc_key).unwrap().is_nil();
                                }
                            }
                        }
                    }
                }
                false
            }
            GcId::UserdataId(id) => {
                // Check if userdata has __gc metamethod
                if let Some(gc_obj) = pool.gc_pool.get(id.0) {
                    if let GcPtrObject::Userdata(ud) = &gc_obj.ptr {
                        let metatable = ud.get_metatable();
                        if let Some(mt_table) = metatable.as_table() {
                            let gc_key = pool.tm_gc.clone();
                            return mt_table.raw_get(&gc_key).is_some()
                                && !mt_table.raw_get(&gc_key).unwrap().is_nil();
                        }
                    }
                }
                false
            }
            GcId::ThreadId(id) => {
                // Threads don't typically have __gc in standard Lua, but check anyway
                if let Some(gc_obj) = pool.gc_pool.get(id.0) {
                    if let GcPtrObject::Thread(_) = &gc_obj.ptr {
                        // TODO: Check if thread has __gc if you support it
                        return false;
                    }
                }
                false
            }
            _ => false, // Other types don't support __gc
        };
        result
    }

    /// Get weak mode for a table (returns None if not weak, or Some((weak_keys, weak_values)))
    fn get_weak_mode(&self, table_id: TableId, pool: &ObjectPool) -> Option<(bool, bool)> {
        let table = pool.get_table(table_id)?;
        let meta_id = table.get_metatable()?;
        let mode_key = pool.tm_mode.clone();
        let metatable = pool.get_table(meta_id)?;
        let weak = metatable.raw_get(&mode_key)?;
        let weak_str = weak.as_str()?;
        let weak_keys = weak_str.contains('k');
        let weak_values = weak_str.contains('v');
        Some((weak_keys, weak_values))
    }

    /// Static helper to convert LuaValue to GcId
    fn value_to_gc_id_static(value: &LuaValue) -> Option<GcId> {
        match value.kind() {
            LuaValueKind::String => value.as_string_id().map(GcId::StringId),
            LuaValueKind::Table => value.as_table_id().map(GcId::TableId),
            LuaValueKind::Function => value.as_function_id().map(GcId::FunctionId),
            LuaValueKind::Thread => value.as_thread_id().map(GcId::ThreadId),
            LuaValueKind::Userdata => value.as_userdata_id().map(GcId::UserdataId),
            _ => None,
        }
    }

    // ============ Weak Table Clearing Functions (Port of Lua 5.5) ============

    /// Port of Lua 5.5's convergeephemerons
    /// Iterate ephemeron tables until convergence
    /// Port of Lua 5.5's convergeephemerons
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// /*
    /// ** Traverse all ephemeron tables propagating marks from keys to values.
    /// ** Repeat until it converges, that is, nothing new is marked. 'dir'
    /// ** inverts the direction of the traversals, trying to speed up
    /// ** convergence on chains in the same table.
    /// */
    /// static void convergeephemerons (global_State *g) {
    ///   int changed;
    ///   int dir = 0;
    ///   do {
    ///     GCObject *w;
    ///     GCObject *next = g->ephemeron;
    ///     g->ephemeron = NULL;
    ///     changed = 0;
    ///     while ((w = next) != NULL) {
    ///       Table *h = gco2t(w);
    ///       next = h->gclist;
    ///       nw2black(h);
    ///       if (traverseephemeron(g, h, dir)) {
    ///         propagateall(g);
    ///         changed = 1;
    ///       }
    ///     }
    ///     dir = !dir;
    ///   } while (changed);
    /// }
    /// ```
    fn converge_ephemerons(&mut self, pool: &mut ObjectPool) {
        let mut changed = true;
        let mut dir = false;
        let mut iteration = 0;
        const MAX_ITERATIONS: usize = 1000;

        while changed && iteration < MAX_ITERATIONS {
            let ephemeron_list = std::mem::take(&mut self.ephemeron);
            self.ephemeron.clear();
            changed = false;
            iteration += 1;

            for table_id in ephemeron_list {
                if let Some(gc_table) = pool.get_mut(table_id.into()) {
                    gc_table.header.make_black();
                }

                let marked = self.traverse_ephemeron_atomic(table_id, pool, dir);
                if marked {
                    while !self.gray.is_empty() {
                        self.propagate_mark(pool);
                    }
                    changed = true;
                }
            }
            
            dir = !dir;
        }
        
        if iteration >= MAX_ITERATIONS {
            eprintln!("WARNING: converge_ephemerons exceeded max iterations");
        }
    }

    /// Traverse ephemeron in atomic phase - returns true if any value was marked
    fn traverse_ephemeron_atomic(&mut self, table_id: TableId, pool: &mut ObjectPool, inv: bool) -> bool {
        let entries = if let Some(table) = pool.get_table_mut(table_id) {
            table.iter_all()
        } else {
            return false;
        };

        let mut marked_any = false;
        let mut has_white_keys = false;
        let mut has_white_white = false;
        
        let mut entry_list: Vec<_> = entries.into_iter().collect();
        if inv {
            entry_list.reverse();
        }

        for (k, v) in entry_list {
            let key_id = Self::value_to_gc_id_static(&k);
            let val_id = Self::value_to_gc_id_static(&v);

            let key_is_cleared = key_id.map_or(false, |id| self.is_cleared(id, pool));
            let val_is_white = val_id.map_or(false, |id| self.is_white(id, pool));

            if key_is_cleared {
                has_white_keys = true;
                if val_is_white {
                    has_white_white = true;
                }
            } else if val_is_white {
                self.mark_value(&v, pool);
                marked_any = true;
            }
        }

        // Port of Lua 5.5 logic:
        // - If has white->white, keep in ephemeron for another convergence pass
        // - If only has white keys (no white->white), move to allweak for later clearbykeys
        // - If no white keys at all, don't add anywhere (table is clean)
        //
        // BUT WAIT: In Lua 5.5, ephemeron tables stay in g->ephemeron until clearbykeys!
        // Looking at lgc.c more carefully:
        //   - convergeephemerons removes from g->ephemeron temporarily
        //   - traverseephemeron(g, h, 1) with atomic=1 calls linkgclist(h, g->allweak) if has white keys
        //   - Then clearbykeys processes BOTH g->ephemeron and g->allweak
        //
        // So the logic is:
        // - white->white: back to ephemeron (need more convergence)
        // - white keys but converged: to allweak (ready for clearing)
        if has_white_white {
            self.ephemeron.push(table_id);
        } else if has_white_keys {
            self.allweak.push(table_id);
        }

        marked_any
    }

    /// Port of Lua 5.5's clearbykeys
    /// Clear entries with unmarked keys from ephemeron and fully weak tables
    fn clear_by_keys(&mut self, pool: &mut ObjectPool) {
        // Clear ephemeron tables
        let ephemeron_list = std::mem::take(&mut self.ephemeron);
        for table_id in ephemeron_list {
            self.clear_table_by_keys(table_id, pool);
        }

        // Clear fully weak tables
        let allweak_list = std::mem::take(&mut self.allweak);
        for table_id in allweak_list {
            self.clear_table_by_keys(table_id, pool);
        }
    }

    /// Clear entries with unmarked keys from a single table
    fn clear_table_by_keys(&mut self, table_id: TableId, pool: &mut ObjectPool) {
        let entries = if let Some(table) = pool.get_table_mut(table_id) {
            table.iter_all()
        } else {
            return;
        };

        let mut keys_to_remove = Vec::new();
        
        for (key, _value) in entries {
            if let Some(key_id) = Self::value_to_gc_id_static(&key) {
                if self.is_cleared(key_id, pool) {
                    keys_to_remove.push(key);
                }
            }
        }
        
        // Remove entries with dead keys
        if let Some(table) = pool.get_table_mut(table_id) {
            for key in keys_to_remove {
                table.raw_set(&key, LuaValue::nil());
            }
        }
    }

    /// Port of Lua 5.5's clearbyvalues
    /// Clear entries with unmarked values from weak value tables
    fn clear_by_values(&mut self, pool: &mut ObjectPool) {
        let weak_list = self.weak.clone();
        for table_id in weak_list {
            self.clear_table_by_values(table_id, pool);
        }

        let allweak_list = self.allweak.clone();
        for table_id in allweak_list {
            self.clear_table_by_values(table_id, pool);
        }
    }
    
    /// Second pass of clear_by_values for resurrected objects
    /// Only process tables NOT in original lists (Lua 5.5: clearbyvalues(g, g->weak, origweak))
    fn clear_by_values_range(&mut self, pool: &mut ObjectPool, origweak: &[TableId], origall: &[TableId]) {
        // Process weak tables added after finalization
        let weak_list = self.weak.clone();
        for table_id in weak_list {
            if !origweak.contains(&table_id) {
                self.clear_table_by_values(table_id, pool);
            }
        }

        // Process allweak tables added after finalization  
        let allweak_list = self.allweak.clone();
        for table_id in allweak_list {
            if !origall.contains(&table_id) {
                self.clear_table_by_values(table_id, pool);
            }
        }
    }

    /// Clear entries with unmarked values from a single table
    fn clear_table_by_values(&mut self, table_id: TableId, pool: &mut ObjectPool) {
        let entries = if let Some(table) = pool.get_table_mut(table_id) {
            table.iter_all()
        } else {
            return;
        };

        let mut keys_to_remove = Vec::new();

        for (key, value) in entries {
            // In Lua weak table semantics:
            // - Strings, numbers, booleans, nil are VALUES (never removed by weak tables)
            // - Only tables, functions, threads, userdata are GC objects
            // So we SKIP strings here
            match value.kind() {
                LuaValueKind::Table | LuaValueKind::Function | 
                LuaValueKind::Thread | LuaValueKind::Userdata => {
                    if let Some(val_id) = Self::value_to_gc_id_static(&value) {
                        if self.is_cleared(val_id, pool) {
                            keys_to_remove.push(key);
                        }
                    }
                }
                _ => {
                    // String, Number, Boolean, Nil: never removed from weak value tables
                }
            }
        }

        // Remove entries with dead values
        if let Some(table) = pool.get_table_mut(table_id) {
            for key in keys_to_remove {
                table.raw_set(&key, LuaValue::nil());
            }
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
        // Lua 5.5 luaC_step:
        // if (!gcrunning(g)) {
        //   if (g->gcstp & GCSTPUSR) luaE_setdebt(g, 20000);
        // } else { ... }
        
        // Check if GC is stopped by user (unless forced)
        if !force && self.gc_stopped {
            // Lua 5.5: set reasonable debt to avoid being called at every check
            self.set_debt(20000);
            return;
        }
        
        // If not forced, check debt
        // Lua 5.5: luaC_condGC only runs step when GCdebt <= 0
        if !force && self.gc_debt > 0 {
            return; // Still have budget, no need to collect
        }

        // Dispatch based on GC mode (like Lua 5.5 luaC_step)
        match self.gc_kind {
            GcKind::Inc | GcKind::GenMajor => self.inc_step(roots, pool),
            GcKind::GenMinor => {
                self.young_collection(roots, pool);
                self.set_minor_debt();
            }
        }
    }

    /// Incremental GC step (like incstep in Lua 5.5)
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// static void incstep (lua_State *L, global_State *g) {
    ///   l_mem stepsize = applygcparam(g, STEPSIZE, 100);
    ///   l_mem work2do = applygcparam(g, STEPMUL, stepsize / cast_int(sizeof(void*)));
    ///   l_mem stres;
    ///   int fast = (work2do == 0);
    ///   do {
    ///     stres = singlestep(L, fast);
    ///     if (stres == step2minor)
    ///       return;
    ///     else if (stres == step2pause || (stres == atomicstep && !fast))
    ///       break;
    ///     else
    ///       work2do -= stres;
    ///   } while (fast || work2do > 0);
    ///   if (g->gcstate == GCSpause)
    ///     setpause(g);
    ///   else
    ///     luaE_setdebt(g, stepsize);
    /// }
    /// ```
    fn inc_step(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // l_mem stepsize = applygcparam(g, STEPSIZE, 100);
        let stepsize = self.apply_param(STEPSIZE, 100);
        
        // l_mem work2do = applygcparam(g, STEPMUL, stepsize / cast_int(sizeof(void*)));
        let ptr_size = std::mem::size_of::<*const ()>() as isize;
        let mut work2do = self.apply_param(STEPMUL, stepsize / ptr_size);
        let initial_work2do = work2do;
        

        
        // int fast = (work2do == 0);
        let fast = work2do == 0;

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

        // Set debt for next step (like Lua 5.5 incstep)

        if self.gc_state == GcState::Pause {
            self.set_pause();
        } else {
            // Lua 5.5: luaE_setdebt(g, stepsize);
            // Set positive debt = buffer before next GC
            self.set_debt(stepsize);
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
                // Set gc_state to Propagate AFTER restart_collection
                // This matches Lua 5.5's logic in singlestep case GCSpause
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
                // The actual finalization happens via pending_actions processed by the VM
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
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// /*
    /// ** mark root set and reset all gray lists, to start a new collection.
    /// ** 'GCmarked' is initialized to count the total number of live bytes
    /// ** during a cycle.
    /// */
    /// static void restartcollection (global_State *g) {
    ///   cleargraylists(g);  // gray = grayagain = weak = allweak = ephemeron = NULL
    ///   g->GCmarked = 0;
    ///   markobject(g, mainthread(g));
    ///   markvalue(g, &g->l_registry);
    ///   markmt(g);  /* mark global metatables */
    ///   markbeingfnz(g);  /* mark any finalizing object left from previous cycle */
    /// }
    /// ```
    /// 
    /// CRITICAL: Lua 5.5 calls cleargraylists which clears ALL gray lists including weak table lists.
    /// This is safe because restartcollection is only called from GCSpause state,
    /// meaning the previous cycle has completely finished (atomic phase cleared weak tables).
    fn restart_collection(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.stats.collection_count += 1;
        
        // CRITICAL: Reset sweep_index when starting a new cycle
        // This ensures the next sweep will scan all objects from the beginning
        self.sweep_index = 0;
        
        // NOTE: Do NOT set gc_state here! It should be set by the caller
        // Lua 5.5's restartcollection does not set gcstate
        
        // Clear all gray lists (like Lua 5.5's cleargraylists)
        self.gray.clear();
        self.grayagain.clear();
        // Lua 5.5 also clears weak table lists here:
        self.weak.clear();
        self.ephemeron.clear();
        self.allweak.clear();

        self.gc_marked = 0;

        // IMPORTANT: current_white was flipped in atomic phase.
        // Objects from last cycle are now "other white" and will be collected if not marked.
        // NO need to flip again or make_all_white - objects are already the right color!

        // Mark roots
        for value in roots.iter() {
            self.mark_value(value, pool);
        }
    }

    /// Mark a value (add to gray list if collectable)
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// #define markvalue(g,o) { checkliveness(mainthread(g),o); \
    ///   if (valiswhite(o)) reallymarkobject(g,gcvalue(o)); }
    /// 
    /// static void reallymarkobject (global_State *g, GCObject *o) {
    ///   g->GCmarked += objsize(o);
    ///   switch (o->tt) {
    ///     case LUA_VSHRSTR:
    ///     case LUA_VLNGSTR: {
    ///       set2black(o);  /* nothing to visit */
    ///       break;
    ///     }
    ///     case LUA_VUPVAL: {
    ///       UpVal *uv = gco2upv(o);
    ///       if (upisopen(uv))
    ///         set2gray(uv);  /* open upvalues are kept gray */
    ///       else
    ///         set2black(uv);  /* closed upvalues are visited here */
    ///       markvalue(g, uv->v.p);
    ///       break;
    ///     }
    ///     // ... tables, closures, threads, userdata, protos -> linkobjgclist(o, g->gray)
    ///   }
    /// }
    /// ```
    /// 
    /// CRITICAL: Only mark WHITE objects. Black objects from previous cycle became
    /// "other white" after color flip, so they WILL be marked again.
    /// Fixed objects (metamethod names) are kept GRAY forever, so they're naturally skipped.
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
                            // Update gc_marked for strings (they don't go through gray list)
                            self.gc_marked += s.size() as isize;
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
                            // Update gc_marked for binaries (they don't go through gray list)
                            self.gc_marked += b.size() as isize;
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

    // ============ Weak Table Traversal Functions (Port of Lua 5.5) ============

    /// Traverse a strong (non-weak) table - mark everything
    fn traverse_strong_table(&mut self, table_id: TableId, pool: &mut ObjectPool) -> isize {
        let (entries, metatable) = if let Some(gc_table) = pool.get_mut(table_id.into()) {
            gc_table.header.make_black();
            let table = match gc_table.ptr.as_table_mut() {
                Some(t) => t,
                None => return 0,
            };
            let entries_vec = table.iter_all();
            (entries_vec, table.get_metatable())
        } else {
            return 0;
        };

        // Mark all entries
        for (k, v) in &entries {
            self.mark_value(k, pool);
            self.mark_value(v, pool);
        }

        // Mark metatable
        if let Some(mt_id) = metatable {
            if let Some(mt) = pool.get_table_value(mt_id) {
                self.mark_value(&mt, pool);
            }
        }

        1 + entries.len() as isize
    }

    /// Port of Lua 5.5's traverseweakvalue
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// /*
    /// ** Traverse a table with weak values and link it to proper list. During
    /// ** propagate phase, keep it in 'grayagain' list, to be revisited in the
    /// ** atomic phase. In the atomic phase, if table has any white value,
    /// ** put it in 'weak' list, to be cleared; otherwise, call 'genlink'
    /// ** to check table age in generational mode.
    /// */
    /// static void traverseweakvalue (global_State *g, Table *h) {
    ///   Node *n, *limit = gnodelast(h);
    ///   int hasclears = (h->asize > 0);
    ///   for (n = gnode(h, 0); n < limit; n++) {
    ///     if (isempty(gval(n)))
    ///       clearkey(n);
    ///     else {
    ///       lua_assert(!keyisnil(n));
    ///       markkey(g, n);
    ///       if (!hasclears && iscleared(g, gcvalueN(gval(n))))
    ///         hasclears = 1;
    ///     }
    ///   }
    ///   if (g->gcstate == GCSpropagate)
    ///     linkgclist(h, g->grayagain);  /* must retraverse it in atomic phase */
    ///   else if (hasclears)
    ///     linkgclist(h, g->weak);  /* has to be cleared later */
    ///   else
    ///     genlink(g, obj2gco(h));
    /// }
    /// ```
    fn traverse_weak_value(&mut self, table_id: TableId, pool: &mut ObjectPool) -> isize {
        let (entries, metatable) = if let Some(gc_table) = pool.get_mut(table_id.into()) {
            gc_table.header.make_black();
            let table = match gc_table.ptr.as_table_mut() {
                Some(t) => t,
                None => return 0,
            };
            (table.iter_all(), table.get_metatable())
        } else {
            return 0;
        };

        // Mark metatable
        if let Some(mt_id) = metatable {
            if let Some(mt) = pool.get_table_value(mt_id) {
                self.mark_value(&mt, pool);
            }
        }

        // Lua 5.5 logic (from lgc.c traverseweakvalue):
        // ```c
        // if (g->gcstate == GCSpropagate)
        //     linkgclist(h, g->grayagain);  /* must retraverse it in atomic phase */
        // else if (hasclears)
        //     linkgclist(h, g->weak);  /* has to be cleared later */
        // ```
        // 
        // CRITICAL: ONLY in Propagate state should we add to grayagain!
        // In Pause/Atomic/other states, we directly mark and add to weak list.
        if self.gc_state == GcState::Propagate {
            self.grayagain.push(GcId::TableId(table_id));
            return 1;
        }

        // In atomic phase, mark keys and check values
        let mut has_white_values = false;

        for (k, v) in &entries {
            // Mark key (strong reference)
            self.mark_value(k, pool);

            // IMPORTANT: After marking the key, the value might also be marked
            // if key and value refer to the same object (common in test cases)
            // So we need to check the CURRENT state from pool, not the snapshot
            if let Some(val_id) = Self::value_to_gc_id_static(v) {
                // Re-check if value is STILL white after marking the key
                if self.is_white(val_id, pool) {
                    has_white_values = true;
                }
            }
        }

        // If has white values, add to weak list for clearing
        if has_white_values {
            self.weak.push(table_id);
        }

        1 + entries.len() as isize
    }

    /// Port of Lua 5.5's traverseephemeron
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// /*
    /// ** Traverse an ephemeron table and link it to proper list. Returns true
    /// ** iff any object was marked during this traversal (which implies that
    /// ** convergence has to continue). During propagation phase, keep table
    /// ** in 'grayagain' list, to be visited again in the atomic phase. In
    /// ** the atomic phase, if table has any white->white entry, it has to
    /// ** be revisited during ephemeron convergence (as that key may turn
    /// ** black). Otherwise, if it has any white key, table has to be cleared
    /// ** (in the atomic phase). In generational mode, some tables
    /// ** must be kept in some gray list for post-processing; this is done
    /// ** by 'genlink'.
    /// */
    /// static int traverseephemeron (global_State *g, Table *h, int inv) {
    ///   int hasclears = 0;
    ///   int hasww = 0;
    ///   unsigned int i;
    ///   unsigned int nsize = sizenode(h);
    ///   int marked = traversearray(g, h);
    ///   for (i = 0; i < nsize; i++) {
    ///     Node *n = inv ? gnode(h, nsize - 1 - i) : gnode(h, i);
    ///     if (isempty(gval(n)))
    ///       clearkey(n);
    ///     else if (iscleared(g, gckeyN(n))) {
    ///       hasclears = 1;
    ///       if (valiswhite(gval(n)))
    ///         hasww = 1;
    ///     }
    ///     else if (valiswhite(gval(n))) {
    ///       marked = 1;
    ///       reallymarkobject(g, gcvalue(gval(n)));
    ///     }
    ///   }
    ///   if (g->gcstate == GCSpropagate)
    ///     linkgclist(h, g->grayagain);
    ///   else if (hasww)
    ///     linkgclist(h, g->ephemeron);
    ///   else if (hasclears)
    ///     linkgclist(h, g->allweak);
    ///   else
    ///     genlink(g, obj2gco(h));
    ///   return marked;
    /// }
    /// ```
    fn traverse_ephemeron(&mut self, table_id: TableId, pool: &mut ObjectPool) -> isize {
        let (entries, metatable) = if let Some(gc_table) = pool.get_mut(table_id.into()) {
            gc_table.header.make_black();
            let table = match gc_table.ptr.as_table_mut() {
                Some(t) => t,
                None => return 0,
            };
            (table.iter_all(), table.get_metatable())
        } else {
            return 0;
        };

        // Mark metatable
        if let Some(mt_id) = metatable {
            if let Some(mt) = pool.get_table_value(mt_id) {
                self.mark_value(&mt, pool);
            }
        }

        // Lua 5.5 logic (from lgc.c traverseephemeron):
        // ```c
        // if (g->gcstate == GCSpropagate)
        //     return propagate;  /* have to propagate again */
        // else {  /* atomic phase */
        //     ...
        // }
        // ```
        // 
        // CRITICAL: ONLY in Propagate state should we return "propagate again"!
        // In Propagate state, we add to grayagain and return.
        // In other states (Pause/Atomic), we proceed to classify entries.
        if self.gc_state == GcState::Propagate {
            self.grayagain.push(GcId::TableId(table_id));
            return 1;
        }

        // Atomic phase: check entries and classify table
        let mut has_white_keys = false;
        let mut has_white_white = false;
        let mut marked_any = false;

        for (k, v) in &entries {
            let key_id = Self::value_to_gc_id_static(k);
            let val_id = Self::value_to_gc_id_static(v);

            // Check if key is cleared (iscleared will mark strings)
            let key_is_cleared = key_id.map_or(false, |id| self.is_cleared(id, pool));
            let val_is_white = val_id.map_or(false, |id| self.is_white(id, pool));

            if key_is_cleared {
                has_white_keys = true;
                if val_is_white {
                    has_white_white = true;
                }
            } else if val_is_white {
                // Key is alive, but value is white - mark the value
                self.mark_value(v, pool);
                marked_any = true;
            }
        }

        // Add to appropriate list
        if has_white_white {
            // Has white->white entries, need convergence
            self.ephemeron.push(table_id);
        } else if has_white_keys {
            // Has white keys but no white->white, just needs clearing
            self.ephemeron.push(table_id);
        }

        // Return whether we marked anything (for convergence check)
        if marked_any {
            1 + entries.len() as isize
        } else {
            1
        }
    }

    /// Port of Lua 5.5's traversetable (case 3: weak keys and values)
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// case 3:  /* all weak; nothing to traverse */
    ///   if (g->gcstate == GCSpropagate)
    ///     linkgclist(h, g->grayagain);  /* must visit again its metatable */
    ///   else
    ///     linkgclist(h, g->allweak);  /* must clear collected entries */
    ///   break;
    /// ```
    fn traverse_fully_weak(&mut self, table_id: TableId, pool: &mut ObjectPool) -> isize {
        let metatable = if let Some(gc_table) = pool.get_mut(table_id.into()) {
            gc_table.header.make_black();
            let table = match gc_table.ptr.as_table_mut() {
                Some(t) => t,
                None => return 0,
            };
            table.get_metatable()
        } else {
            return 0;
        };

        // Don't mark keys or values - they're all weak

        // Mark metatable
        if let Some(mt_id) = metatable {
            if let Some(mt) = pool.get_table_value(mt_id) {
                self.mark_value(&mt, pool);
            }
        }

        // Lua 5.5 logic (from lgc.c traverseallweak):
        // ```c
        // if (g->gcstate == GCSpropagate)
        //     linkgclist(h, g->grayagain);
        // else
        //     linkgclist(h, g->allweak);
        // ```
        // 
        // CRITICAL: ONLY in Propagate state should we add to grayagain!
        // In other states, directly add to allweak list.
        if self.gc_state == GcState::Propagate {
            self.grayagain.push(GcId::TableId(table_id));
        } else {
            self.allweak.push(table_id);
        }

        1
    }

    /// Check if an object is white
    fn is_white(&self, gc_id: GcId, pool: &ObjectPool) -> bool {
        if let Some(obj) = pool.gc_pool.get(gc_id.index()) {
            obj.header.is_white()
        } else {
            true // Non-existent objects are considered "dead"
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
                // Port of Lua 5.5's traversetable with weak table handling
                // Check weak mode first to decide how to traverse
                
                let weak_mode = self.get_weak_mode(id, pool);

                match weak_mode {
                    None | Some((false, false)) => {
                        // Regular table (or invalid weak mode) - mark everything
                        self.traverse_strong_table(id, pool)
                    }
                    Some((false, true)) => {
                        // Weak values only (__mode = 'v')
                        self.traverse_weak_value(id, pool)
                    }
                    Some((true, false)) => {
                        // Weak keys only (__mode = 'k') - ephemeron
                        self.traverse_ephemeron(id, pool)
                    }
                    Some((true, true)) => {
                        // Both weak (__mode = 'kv') - fully weak
                        self.traverse_fully_weak(id, pool)
                    }
                }
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
        // Lua 5.5's atomic phase does:
        // 1. Set gcstate = GCSatomic
        // 2. Mark running thread, registry, global metatables
        // 3. propagateall(g) - empty gray list
        // 4. remarkupvals(g) - handle thread upvalues
        // 5. propagateall(g) - propagate changes
        // 6. g->gray = grayagain; propagateall(g) - process grayagain (WEAK TABLES!)
        // 7. convergeephemerons(g) - converge ephemeron tables
        // 8. clearbyvalues(weak & allweak) - first pass
        // 9. separatetobefnz + markbeingfnz + propagateall - handle finalizers
        // 10. convergeephemerons(g) - second pass after resurrection
        // 11. clearbykeys(ephemeron & allweak) - clear dead keys
        // 12. clearbyvalues(weak & allweak, origweak & origall) - clear resurrected
        // 13. g->currentwhite = otherwhite(g) - FLIP WHITE COLOR
        
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
            let size = self.mark_one(gc_id, pool);
            // Update gc_marked for objects processed from grayagain
            self.gc_marked += size;
        }

        // Propagate again to handle anything pushed by grayagain processing
        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }

        // Port of Lua 5.5's atomic() phase for weak tables
        // At this point, all strongly accessible objects are marked.

        // First convergence of ephemeron tables
        self.converge_ephemerons(pool);

        // Clear weak values (first pass)
        self.clear_by_values(pool);
        
        // Save original lists before finalization (Lua 5.5: origweak = g->weak; origall = g->allweak;)
        let origweak = self.weak.clone();
        let origall = self.allweak.clone();

        // TODO: Handle finalizers (separatetobefnz, markbeingfnz)
        // For now we skip finalization

        // Second convergence after potential resurrection
        self.converge_ephemerons(pool);

        // Clear weak keys from ephemeron and allweak tables
        self.clear_by_keys(pool);

        // Clear weak values (second pass, only for resurrected objects)
        // Lua 5.5: clearbyvalues(g, g->weak, origweak); clearbyvalues(g, g->allweak, origall);
        // This clears new entries added after finalization
        // Since we skip finalization, this should be a no-op, but keep it for correctness
        self.clear_by_values_range(pool, &origweak, &origall);

        // Flip white color for next cycle
        // After this, objects with old white are considered dead
        self.current_white ^= 1;
    }

    /// Enter sweep phase (like entersweep in Lua 5.5)
    pub fn enter_sweep(&mut self, pool: &mut ObjectPool) {
        self.gc_state = GcState::SwpAllGc;
        self.sweep_index = 0; // Reset sweep position
        
        // Collect all object IDs at the start of sweep phase
        // This avoids iterating through sparse Vec slots (capacity >> len)
        // New objects created during sweep will have current_white, so won't be collected
        self.sweep_ids = pool.gc_pool.iter().map(|(id, _)| id).collect();
        self.sweep_target = self.sweep_ids.len();
        


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

        // Use sweep_ids collected at start of sweep phase
        // This avoids iterating through sparse Vec slots
        let sweep_end = self.sweep_target;

        let mut dead_ids = Vec::new();
        let mut to_finalize = Vec::new();

        // Continue from where we left off in sweep_ids
        while self.sweep_index < sweep_end && count < max_sweep {
            let gc_id = self.sweep_ids[self.sweep_index];

            // Check if this object still exists (it might have been collected already)
            if let Some(obj) = pool.gc_pool.get_mut(gc_id.index()) {
                // Check if object is dead (other white and not fixed)
                if !obj.header.is_fixed() && obj.header.is_dead(other_white) {
                    // Dead object - mark for removal or finalization
                    
                    // Check if object needs finalization (__gc metamethod)
                    // Also check if it's already been finalized (FINALIZED flag)
                    let already_finalized = obj.header.to_finalize();
                    let needs_fin = !already_finalized && self.needs_finalization(gc_id, pool);
                    if needs_fin {
                        // Mark object as pending finalization (FINALIZED flag)
                        // This prevents __gc from being called twice
                        if let Some(obj) = pool.get_mut(gc_id) {
                            obj.header.set_finalized();
                        }
                        
                        to_finalize.push(gc_id);

                        // Resurrect object: mark it and all its references (including metatable)
                        // This ensures the object and everything it references survives this GC cycle
                        // so the finalizer can access them safely
                        self.mark_one(gc_id, pool);
                    } else {
                        dead_ids.push(gc_id);
                    }
                } else if !obj.header.is_fixed() {
                    // Lua 5.5 sweeplist logic: Surviving objects are reset to current white!
                    // ```c
                    // else {  /* change mark to 'white' and age to 'new' */
                    //     curr->marked = cast_byte((marked & ~maskgcbits) | white | G_NEW);
                    // }
                    // ```
                    // This is CRITICAL: Without this, BLACK objects stay BLACK across cycles
                    // and won't be remarked in the next cycle!
                    obj.header.make_white(self.current_white);
                    obj.header.set_age(G_NEW);
                }
            }

            self.sweep_index += 1;
            count += 1;
        }

        // Add to pending actions
        self.pending_actions.to_finalize.extend(to_finalize);

        // Actually remove dead objects (those without finalizers)
        // BUT: filter out any objects that were resurrected by mark_one during finalization setup
        // This can happen when a dead object's metatable was collected before the object with __gc
        // Lua 5.5 lmem.c luaM_free_:
        //   g->GCdebt += cast(l_mem, osize);
        // 释放时增加GCdebt，不修改GCtotalbytes！
        // 不变量：真实内存 = GCtotalbytes - GCdebt
        // 释放时：debt增加size，totalbytes不变，所以真实内存减少size
        let dead_count = dead_ids.len();
        let mut freed_bytes = 0usize;
        for gc_id in &dead_ids {
            // Check if the object was resurrected (no longer dead)
            let still_dead = if let Some(obj) = pool.get(*gc_id) {
                obj.header.is_dead(other_white)
            } else {
                false // Already removed somehow
            };
            
            if still_dead {
                let size = pool.remove(*gc_id);
                if size > 0 {
                    self.gc_debt += size as isize;  // 释放增加debt
                    self.stats.bytes_freed += size;
                    self.stats.objects_collected += 1;
                    freed_bytes += size;
                }
            }
        }
        
        let _ = (dead_count, freed_bytes); // suppress unused warnings

        // Return true if we've reached the sweep target (set at start of sweep phase)
        self.sweep_index >= sweep_end
    }

    pub fn set_pause(&mut self) {
        // Lua 5.5 lgc.c setpause:
        // l_mem threshold = applygcparam(g, PAUSE, g->GCmarked);
        // l_mem debt = threshold - gettotalbytes(g);
        // if (debt < 0) debt = 0;
        // luaE_setdebt(g, debt);
        // 
        // Key: threshold based on GCmarked (live bytes after collection), not total
        // gettotalbytes(g) = GCtotalbytes - GCdebt
        let threshold = self.apply_param(PAUSE, self.gc_marked);
        let real_bytes = self.total_bytes - self.gc_debt;  // gettotalbytes
        let mut debt = threshold - real_bytes;
        
        if debt < 0 {
            debt = 0; // Don't allow negative debt after pause
        }
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
    /// Port of Lua 5.5's fullgen (partial - we use youngcollection for GenMinor)
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// /*
    /// ** Does a young collection. First, mark 'OLD1' objects. Then does the
    /// ** atomic step. Then, check whether to continue in minor mode. If so,
    /// ** sweep all lists and advance pointers. Finally, finish the collection.
    /// */
    /// static void youngcollection (lua_State *L, global_State *g) {
    ///   l_mem addedold1 = 0;
    ///   l_mem marked = g->GCmarked;
    ///   GCObject **psurvival;
    ///   GCObject *dummy;
    ///   lua_assert(g->gcstate == GCSpropagate);
    ///   if (g->firstold1) {
    ///     markold(g, g->firstold1, g->reallyold);
    ///     g->firstold1 = NULL;
    ///   }
    ///   markold(g, g->finobj, g->finobjrold);
    ///   markold(g, g->tobefnz, NULL);
    ///
    ///   atomic(L);  /* will lose 'g->marked' */
    ///   ...
    /// }
    /// ```
    /// 
    /// Port of Lua 5.5's fullgen / youngcollection
    /// 
    /// From lgc.c (Lua 5.5):
    /// ```c
    /// /*
    /// ** Does a young collection. First, mark 'OLD1' objects. Then does the
    /// ** atomic step. Then, check whether to continue in minor mode. If so,
    /// ** sweep all lists and advance pointers. Finally, finish the collection.
    /// */
    /// static void youngcollection (lua_State *L, global_State *g) {
    ///   l_mem addedold1 = 0;
    ///   l_mem marked = g->GCmarked;
    ///   GCObject **psurvival;
    ///   GCObject *dummy;
    ///   lua_assert(g->gcstate == GCSpropagate);
    ///   if (g->firstold1) {
    ///     markold(g, g->firstold1, g->reallyold);
    ///     g->firstold1 = NULL;
    ///   }
    ///   markold(g, g->finobj, g->finobjrold);
    ///   markold(g, g->tobefnz, NULL);
    ///
    ///   atomic(L);  /* will lose 'g->marked' */
    ///   ...
    /// }
    /// ```
    /// 
    /// NOTE: In generational mode, full collection does:
    /// 1. restart_collection (marks roots, weak tables go to grayagain)
    /// 2. propagate gray list
    /// 3. Process grayagain list (THIS WAS MISSING!)
    /// 4. propagate again
    /// 5. converge ephemerons
    /// 6. clear weak tables
    pub fn full_generation(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        // If we're in Pause state, we need to do the state transition ourselves
        if self.gc_state == GcState::Pause {
            // CRITICAL: Call restart_collection WHILE STILL IN PAUSE STATE!
            // Lua 5.5's singlestep calls restartcollection() while gcstate==GCSpause,
            // THEN sets gcstate=GCSpropagate.
            self.restart_collection(roots, pool);
            
            // NOW transition to Propagate state (like Lua 5.5's singlestep)
            self.gc_state = GcState::Propagate;
        }
        
        // Step 2: Propagate gray list (weak tables will be added to grayagain)
        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }
        
        // Step 3: Call atomic phase (like youngcollection does)
        // atomic() will process grayagain, converge ephemerons, and clear weak tables
        self.atomic(roots, pool);

        // Step 4: Sweep
        self.enter_sweep(pool);
        self.run_until_state(GcState::CallFin, roots, pool);
        self.run_until_state(GcState::Pause, roots, pool);

        // set_pause uses total_bytes (actual memory after sweep) as base
        self.set_pause();
    }

    /// Set minor debt for generational mode
    /// Port of Lua 5.5's setminordebt:
    /// ```c
    /// static void setminordebt (global_State *g) {
    ///   luaE_setdebt(g, applygcparam(g, MINORMUL, g->GCmajorminor));
    /// }
    /// ```
    fn set_minor_debt(&mut self) {
        // Use gc_majorminor as base (number of bytes from last major collection)
        // If not set yet, use a reasonable default
        let base = if self.gc_majorminor > 0 {
            self.gc_majorminor
        } else {
            // Use current gc_marked or a minimum base
            self.gc_marked.max(64 * 1024) // 64KB minimum
        };
        let debt = self.apply_param(MINORMUL, base);
        self.set_debt(debt);
    }

    /// Young collection for generational mode
    /// Port of Lua 5.5's youngcollection:
    /// ```c
    /// static void youngcollection (lua_State *L, global_State *g) {
    ///   lua_assert(g->gcstate == GCSpropagate);
    ///   if (g->firstold1) {
    ///     markold(g, g->firstold1, g->reallyold);
    ///     g->firstold1 = NULL;
    ///   }
    ///   markold(g, g->finobj, g->finobjrold);
    ///   markold(g, g->tobefnz, NULL);
    ///   atomic(L);  /* will lose 'g->marked' */
    ///   /* sweep nursery and get a pointer to its last live element */
    ///   g->gcstate = GCSswpallgc;
    ///   psurvival = sweepgen(L, g, &g->allgc, g->survival, &g->firstold1, &addedold1);
    ///   ...
    ///   finishgencycle(L, g);
    /// }
    /// ```
    fn young_collection(&mut self, roots: &[LuaValue], pool: &mut ObjectPool) {
        self.stats.minor_collections += 1;

        // Start collection: mark roots
        self.restart_collection(roots, pool);
        self.gc_state = GcState::Propagate;

        // Propagate all marks
        while !self.gray.is_empty() {
            self.propagate_mark(pool);
        }

        // CRITICAL: Call atomic phase!
        // This flips current_white so sweep can identify dead objects
        self.atomic(roots, pool);

        // Enter sweep phase
        self.enter_sweep(pool);
        
        // Complete sweep (fast mode = sweep everything)
        while !self.sweep_step(pool, true) {
            // Continue sweeping until complete
        }

        // Return to pause state, ready for next cycle
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
            // In propagate/atomic phase: link into grayagain for re-traversal in current cycle
            // In pause/sweep phase: link into gray for next cycle
            let target_list = if self.gc_state == GcState::Pause || self.gc_state.is_sweep_phase() {
                &mut self.gray
            } else {
                &mut self.grayagain
            };

            if !target_list.contains(&o_id) {
                target_list.push(o_id);
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
#[derive(Debug)]
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
