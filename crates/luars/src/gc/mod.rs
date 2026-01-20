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

mod gc_kind;
mod gc_object;
mod object_allocator;
mod string_interner;

use crate::lua_value::{Chunk, LuaValue};
pub use gc_kind::*;
pub use gc_object::*;
pub use object_allocator::*;

/// Actions that GC needs VM to perform after a GC step
/// This allows GC to mark objects for finalization
/// Weak tables are now cleaned directly during GC atomic phase
#[derive(Default)]
pub struct GcActions {
    /// Objects that need their __gc finalizer called
    pub to_finalize: Vec<GcObjectPtr>,
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
        0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
        5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
        6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
        7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
        7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
        8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
        8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
        8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
        8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
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
    // General GC pool for all objects
    pub(crate) gc_pool: GcPool,
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
    pub gray: Vec<GcObjectPtr>,

    /// Objects to be revisited at atomic phase
    pub grayagain: Vec<GcObjectPtr>,

    // === Weak table lists (Port of Lua 5.5) ===
    /// Weak value tables (only values are weak)
    pub weak: Vec<TablePtr>,

    /// Ephemeron tables (keys are weak, but key存活则value存活)
    pub ephemeron: Vec<TablePtr>,

    /// Fully weak tables (both keys and values are weak)
    pub allweak: Vec<TablePtr>,

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

    pub tm_gc: LuaValue,

    pub tm_mode: LuaValue,
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
            gc_pool: GcPool::new(),
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
            survival: None,
            old1: None,
            reallyold: None,
            firstold1: None,
            stats: GcStats::default(),
            tm_gc: LuaValue::nil(),
            tm_mode: LuaValue::nil(),
        }
    }

    /// Change to incremental mode (like minor2inc in Lua 5.5)
    pub fn change_to_incremental_mode(&mut self, pool: &mut ObjectAllocator) {
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
    pub fn track_size(&mut self, size: usize) {
        let size_signed = size as isize;

        // Lua 5.5 lmem.c luaM_malloc_:
        self.gc_debt -= size_signed; // 分配减少debt
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

        let m = (p & 0xF) as isize; // mantissa
        let e = (p >> 4) as i32; // exponent

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

    /// Check if a GcPtr represents a dead object (will be collected)
    /// Used by weak table cleanup to identify dead keys/values
    pub fn is_object_dead(&self, gc_ptr: GcObjectPtr) -> bool {
        if let Some(header) = gc_ptr.header() {
            // Fixed objects are never dead
            // if header.is_fixed() {
            //     return false;
            // }
            // Calculate other_white (the white that will be collected)
            let other_white = GcHeader::otherwhite(self.current_white);
            // Object is dead if it's marked with other_white
            header.is_dead(other_white)
        } else {
            // Object doesn't exist = dead
            true
        }
    }

    /// Port of Lua 5.5's iscleared function from lgc.c
    /// Check if an object is cleared (should be removed from weak table)
    /// For strings: marks them black and returns false (strings are 'values', never weak)
    /// For other objects: returns true if white (will be collected)
    fn is_cleared(&mut self, gc_ptr: GcObjectPtr) -> bool {
        match gc_ptr.kind() {
            GcObjectKind::String | GcObjectKind::Binary => {
                // Strings are 'values', so are never weak
                // Mark the string black directly (strings have nothing to visit)
                // Port of: markobject(g,o) -> reallymarkobject -> set2black for strings
                if let Some(header) = gc_ptr.header_mut() {
                    if header.is_white() {
                        let size = header.size;
                        header.make_black();
                        self.gc_marked += size as isize;
                    }
                }

                false
            }
            _ => {
                // For other objects, check if they're white (will be collected)
                if let Some(header) = gc_ptr.header() {
                    header.is_white()
                } else {
                    true
                }
            }
        }
    }

    /// Check if an object needs finalization (__gc metamethod)
    /// Only tables, userdata, and threads can have __gc
    fn needs_finalization(&self, gc_ptr: GcObjectPtr) -> bool {
        let metatable = match gc_ptr {
            GcObjectPtr::Table(table_ptr) => table_ptr.as_ref().data.get_metatable(),
            GcObjectPtr::Userdata(ud_ptr) => ud_ptr.as_ref().data.get_metatable(),
            _ => return false, // Other types don't support __gc
        };

        if let Some(metatable) = metatable {
            let gc_key = self.tm_gc;
            if let Some(mt_table) = metatable.as_table() {
                if let Some(gc_field) = mt_table.raw_get(&gc_key) {
                    return !gc_field.is_nil();
                }
            }
        }

        false
    }

    /// Get weak mode for a table (returns None if not weak, or Some((weak_keys, weak_values)))
    fn get_weak_mode(&self, table_ptr: TablePtr) -> Option<(bool, bool)> {
        let table = &table_ptr.as_ref().data;
        let metatable_val = table.get_metatable()?;
        let metatable = metatable_val.as_table()?;
        let mode_key = self.tm_mode;
        let weak = metatable.raw_get(&mode_key)?;
        let weak_str = weak.as_str()?;
        let weak_keys = weak_str.contains('k');
        let weak_values = weak_str.contains('v');
        Some((weak_keys, weak_values))
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
    fn converge_ephemerons(&mut self) {
        let mut changed = true;
        let mut dir = false;
        let mut iteration = 0;
        const MAX_ITERATIONS: usize = 1000;

        while changed && iteration < MAX_ITERATIONS {
            let ephemeron_list = std::mem::take(&mut self.ephemeron);
            self.ephemeron.clear();
            changed = false;
            iteration += 1;

            for table_ptr in ephemeron_list {
                table_ptr.as_mut_ref().header.make_black();

                let marked = self.traverse_ephemeron_atomic(table_ptr, dir);
                if marked {
                    while !self.gray.is_empty() {
                        self.propagate_mark();
                    }
                    changed = true;
                }
            }

            dir = !dir;
        }
    }

    /// Traverse ephemeron in atomic phase - returns true if any value was marked
    fn traverse_ephemeron_atomic(&mut self, table_ptr: TablePtr, inv: bool) -> bool {
        let entries = table_ptr.as_ref().data.iter_all();

        let mut marked_any = false;
        let mut has_white_keys = false;
        let mut has_white_white = false;

        let mut entry_list: Vec<_> = entries.into_iter().collect();
        if inv {
            entry_list.reverse();
        }

        for (k, v) in entry_list {
            let key_ptr = k.as_gc_ptr();
            let val_ptr = v.as_gc_ptr();

            let key_is_cleared = key_ptr.map_or(false, |ptr| self.is_cleared(ptr));
            let val_is_white = val_ptr.map_or(false, |ptr| self.is_white(ptr));

            if key_is_cleared {
                has_white_keys = true;
                if val_is_white {
                    has_white_white = true;
                }
            } else if val_is_white {
                self.mark_value(&v);
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
            self.ephemeron.push(table_ptr);
        } else if has_white_keys {
            self.allweak.push(table_ptr);
        }

        marked_any
    }

    /// Port of Lua 5.5's clearbykeys
    /// Clear entries with unmarked keys from ephemeron and fully weak tables
    fn clear_by_keys(&mut self) {
        // Clear ephemeron tables
        let ephemeron_list = std::mem::take(&mut self.ephemeron);
        for table_ptr in ephemeron_list {
            self.clear_table_by_keys(table_ptr);
        }

        // Clear fully weak tables
        let allweak_list = std::mem::take(&mut self.allweak);
        for table_ptr in allweak_list {
            self.clear_table_by_keys(table_ptr);
        }
    }

    /// Clear entries with unmarked keys from a single table
    fn clear_table_by_keys(&mut self, table_ptr: TablePtr) {
        let entries = table_ptr.as_ref().data.iter_keys();

        let mut keys_to_remove = Vec::new();

        for key in entries {
            if !key.is_string()
                && let Some(key_ptr) = key.as_gc_ptr()
            {
                if self.is_cleared(key_ptr) {
                    keys_to_remove.push(key);
                }
            }
        }

        // Remove entries with dead keys
        let table = &mut table_ptr.as_mut_ref().data;
        for key in keys_to_remove {
            table.raw_set(&key, LuaValue::nil());
        }
    }

    /// Port of Lua 5.5's clearbyvalues
    /// Clear entries with unmarked values from weak value tables
    fn clear_by_values(&mut self) {
        let weak_list = self.weak.clone();
        for table_ptr in weak_list {
            self.clear_table_by_values(table_ptr);
        }

        let allweak_list = self.allweak.clone();
        for table_ptr in allweak_list {
            self.clear_table_by_values(table_ptr);
        }
    }

    /// Second pass of clear_by_values for resurrected objects
    /// Only process tables NOT in original lists (Lua 5.5: clearbyvalues(g, g->weak, origweak))
    fn clear_by_values_range(&mut self, origweak: &[TablePtr], origall: &[TablePtr]) {
        // Process weak tables added after finalization
        let weak_list = self.weak.clone();
        for table_id in weak_list {
            if !origweak.contains(&table_id) {
                self.clear_table_by_values(table_id);
            }
        }

        // Process allweak tables added after finalization
        let allweak_list = self.allweak.clone();
        for table_id in allweak_list {
            if !origall.contains(&table_id) {
                self.clear_table_by_values(table_id);
            }
        }
    }

    /// Clear entries with unmarked values from a single table
    fn clear_table_by_values(&mut self, table_ptr: TablePtr) {
        let entries = table_ptr.as_ref().data.iter_all();

        let mut keys_to_remove = Vec::new();

        for (key, value) in entries {
            if !value.is_string()
                && let Some(val_ptr) = value.as_gc_ptr()
            {
                if self.is_cleared(val_ptr) {
                    keys_to_remove.push(key);
                }
            }
        }

        // Remove entries with dead values
        let table = &mut table_ptr.as_mut_ref().data;
        for key in keys_to_remove {
            table.raw_set(&key, LuaValue::nil());
        }
    }

    // ============ Core GC Implementation ============

    /// Main GC step function (like luaC_step in Lua 5.5)
    /// Actions are accumulated in pending_actions, retrieve with take_pending_actions()
    pub fn step(&mut self, roots: &[LuaValue], pool: &mut ObjectAllocator) {
        self.step_internal(roots, pool, false)
    }

    /// Internal step function with force parameter
    /// If force=true, ignore gc_stopped flag (used by collectgarbage("step"))
    pub fn step_internal(&mut self, roots: &[LuaValue], pool: &mut ObjectAllocator, force: bool) {
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
    fn inc_step(&mut self, roots: &[LuaValue], pool: &mut ObjectAllocator) {
        // l_mem stepsize = applygcparam(g, STEPSIZE, 100);
        let stepsize = self.apply_param(STEPSIZE, 100);

        // l_mem work2do = applygcparam(g, STEPMUL, stepsize / cast_int(sizeof(void*)));
        let ptr_size = std::mem::size_of::<*const ()>() as isize;
        let mut work2do = self.apply_param(STEPMUL, stepsize / ptr_size);
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
    fn single_step(
        &mut self,
        roots: &[LuaValue],
        pool: &mut ObjectAllocator,
        fast: bool,
    ) -> StepResult {
        if self.gc_stopem {
            return StepResult::Work(0);
        }

        self.gc_stopem = true;

        let result = match self.gc_state {
            GcState::Pause => {
                self.restart_collection(roots);
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
                    let work = self.propagate_mark();
                    StepResult::Work(work)
                }
            }
            GcState::EnterAtomic => {
                self.atomic(roots);
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
    fn restart_collection(&mut self, roots: &[LuaValue]) {
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
            self.mark_value(value);
        }

        // NOTE: Open upvalues are marked separately by VM calling mark_open_upvalues
        // This is because we need thread access to get open_upvalues list
    }

    /// Mark open upvalues from a thread (like Lua 5.5's remarkupvals)
    /// Open upvalues must be kept gray because their values can change
    ///
    /// Port of Lua 5.5's remarkupvals:
    /// ```c
    /// for (uv = thread->openupval; uv != NULL; uv = uv->u.open.next) {
    ///   if (!iswhite(uv)) {
    ///     markvalue(g, uv->v.p);  // mark the value the upvalue points to
    ///   }
    /// }
    /// ```
    pub fn mark_open_upvalues(&mut self, upvalues: &[UpvaluePtr], state: &crate::lua_vm::LuaState) {
        for upval_ptr in upvalues {
            let gc_upval = upval_ptr.as_mut_ref();
            let header = &mut gc_upval.header;

            if header.is_white() {
                // Lua 5.5: open upvalues are kept gray, not black
                header.make_gray();
                self.gray.push(upval_ptr.clone().into());
            }

            // CRITICAL: Mark the value that the open upvalue points to
            // This is the Lua 5.5 remarkupvals behavior
            if let Some(stack_index) = gc_upval.data.get_stack_index() {
                if let Some(value) = state.stack().get(stack_index) {
                    self.mark_value(value);
                }
            }
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
    fn mark_value(&mut self, value: &LuaValue) {
        let Some(gc_ptr) = value.as_gc_ptr() else {
            return;
        };

        match gc_ptr {
            GcObjectPtr::Table(table_ptr) => {
                let header = &mut table_ptr.as_mut_ref().header;

                // Only mark white objects (fixed objects are gray, so they're skipped)
                if header.is_white() {
                    header.make_gray();
                    self.gray.push(table_ptr.into());
                }
            }
            GcObjectPtr::Function(func_ptr) => {
                let header = &mut func_ptr.as_mut_ref().header;
                if header.is_white() {
                    header.make_gray();
                    self.gray.push(func_ptr.into());
                }
            }
            GcObjectPtr::String(string_ptr) => {
                let header = &mut string_ptr.as_mut_ref().header;
                if header.is_white() {
                    header.make_black(); // Strings are leaves
                    // Update gc_marked for strings (they don't go through gray list)
                    self.gc_marked += header.size as isize;
                }
            }
            GcObjectPtr::Binary(binary_ptr) => {
                let header = &mut binary_ptr.as_mut_ref().header;
                if header.is_white() {
                    header.make_black(); // Binaries are leaves
                    // Update gc_marked for binaries (they don't go through gray list)
                    self.gc_marked += header.size as isize;
                }
            }
            GcObjectPtr::Userdata(userdata_ptr) => {
                let header = &mut userdata_ptr.as_mut_ref().header;
                if header.is_white() {
                    header.make_gray();
                    self.gray.push(userdata_ptr.into());
                }
            }
            GcObjectPtr::Thread(thread_ptr) => {
                let header = &mut thread_ptr.as_mut_ref().header;
                if header.is_white() {
                    header.make_gray();
                    self.gray.push(thread_ptr.into());
                }
            }
            GcObjectPtr::Upvalue(upval_ptr) => {
                let header = &mut upval_ptr.as_mut_ref().header;
                if header.is_white() {
                    header.make_gray();
                    self.gray.push(upval_ptr.into());
                }
            }
        }
    }

    /// Mark all constants in a chunk and its nested chunks (like Lua 5.5's traverseproto)
    fn mark_chunk_constants(&mut self, chunk: &Chunk) {
        // Mark all constants in this chunk
        for constant in &chunk.constants {
            self.mark_value(constant);
        }

        // Recursively mark constants in child protos (nested functions)
        for child_chunk in &chunk.child_protos {
            self.mark_chunk_constants(child_chunk);
        }
    }

    // ============ Weak Table Traversal Functions (Port of Lua 5.5) ============

    /// Traverse a strong (non-weak) table - mark everything
    fn traverse_strong_table(&mut self, table_ptr: TablePtr) -> isize {
        let gc_table = table_ptr.as_mut_ref();
        gc_table.header.make_black();
        let table = &gc_table.data;
        let entries_vec = table.iter_all();

        // Mark all entries
        for (k, v) in &table.iter_all() {
            self.mark_value(k);
            self.mark_value(v);
        }

        // Mark metatable
        if let Some(metatable) = table.get_metatable() {
            self.mark_value(&metatable);
        }

        1 + entries_vec.len() as isize
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
    fn traverse_weak_value(&mut self, table_ptr: TablePtr) -> isize {
        let gc_table = table_ptr.as_mut_ref();
        gc_table.header.make_black();
        let table = &gc_table.data;
        // Mark metatable
        if let Some(metatable) = table.get_metatable() {
            self.mark_value(&metatable);
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
            self.grayagain.push(table_ptr.into());
            return 1;
        }

        // In atomic phase, mark keys and check values
        let mut has_white_values = false;

        let entries = table.iter_all();
        for (k, v) in &entries {
            // Mark key (strong reference)
            self.mark_value(k);

            // IMPORTANT: After marking the key, the value might also be marked
            // if key and value refer to the same object (common in test cases)
            // So we need to check the CURRENT state from pool, not the snapshot
            if let Some(val_ptr) = v.as_gc_ptr() {
                // Re-check if value is STILL white after marking the key
                if self.is_white(val_ptr) {
                    has_white_values = true;
                }
            }
        }

        // If has white values, add to weak list for clearing
        if has_white_values {
            self.weak.push(table_ptr.into());
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
    fn traverse_ephemeron(&mut self, table_ptr: TablePtr) -> isize {
        let gc_table = table_ptr.as_mut_ref();
        gc_table.header.make_black();
        let table = &gc_table.data;

        // Mark metatable
        if let Some(metatable) = table.get_metatable() {
            self.mark_value(&metatable);
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
            self.grayagain.push(table_ptr.into());
            return 1;
        }

        // Atomic phase: check entries and classify table
        let mut has_white_keys = false;
        let mut has_white_white = false;
        let mut marked_any = false;

        let entries = table.iter_all();
        for (k, v) in &entries {
            let key_ptr = k.as_gc_ptr();
            let val_ptr = v.as_gc_ptr();

            // Check if key is cleared (iscleared will mark strings)
            let key_is_cleared = key_ptr.map_or(false, |ptr| self.is_cleared(ptr));
            let val_is_white = val_ptr.map_or(false, |ptr| self.is_white(ptr));
            if key_is_cleared {
                has_white_keys = true;
                if val_is_white {
                    has_white_white = true;
                }
            } else if val_is_white {
                // Key is alive, but value is white - mark the value
                self.mark_value(v);
                marked_any = true;
            }
        }

        // Add to appropriate list
        if has_white_white {
            // Has white->white entries, need convergence
            self.ephemeron.push(table_ptr.into());
        } else if has_white_keys {
            // Has white keys but no white->white, just needs clearing
            self.ephemeron.push(table_ptr.into());
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
    fn traverse_fully_weak(&mut self, table_ptr: TablePtr) -> isize {
        let gc_table = table_ptr.as_mut_ref();
        gc_table.header.make_black();
        let table = &gc_table.data;

        // Don't mark keys or values - they're all weak

        // Mark metatable
        if let Some(metatable) = table.get_metatable() {
            self.mark_value(&metatable);
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
            self.grayagain.push(table_ptr.into());
        } else {
            self.allweak.push(table_ptr.into());
        }

        1
    }

    /// Check if an object is white
    fn is_white(&self, gc_ptr: GcObjectPtr) -> bool {
        if let Some(header) = gc_ptr.header() {
            header.is_white()
        } else {
            false
        }
    }

    /// Propagate mark for one gray object (like propagatemark in Lua 5.5)
    fn propagate_mark(&mut self) -> isize {
        if let Some(gc_ptr) = self.gray.pop() {
            self.mark_one(gc_ptr);
            // Use the size stored in GcObject (same as track_object and sweep)
            // This ensures consistency across all GC operations
            let size = gc_ptr.header().map(|it| it.size as isize).unwrap_or(0);
            self.gc_marked += size;
            size
        } else {
            0
        }
    }

    /// Mark one object and traverse its references
    /// Like Lua 5.5's propagatemark: "nw2black(o);" then traverse
    /// Sets object to BLACK before traversing children
    fn mark_one(&mut self, gc_ptr: GcObjectPtr) -> isize {
        match gc_ptr {
            GcObjectPtr::Table(table_ptr) => {
                // Port of Lua 5.5's traversetable with weak table handling
                // Check weak mode first to decide how to traverse

                let weak_mode = self.get_weak_mode(table_ptr);

                match weak_mode {
                    None | Some((false, false)) => {
                        // Regular table (or invalid weak mode) - mark everything
                        self.traverse_strong_table(table_ptr)
                    }
                    Some((false, true)) => {
                        // Weak values only (__mode = 'v')
                        self.traverse_weak_value(table_ptr)
                    }
                    Some((true, false)) => {
                        // Weak keys only (__mode = 'k') - ephemeron
                        self.traverse_ephemeron(table_ptr)
                    }
                    Some((true, true)) => {
                        // Both weak (__mode = 'kv') - fully weak
                        self.traverse_fully_weak(table_ptr)
                    }
                }
            }
            GcObjectPtr::Function(func_ptr) => {
                // Mark the function black and get references to data we need
                // (Fixed functions should never reach here - they stay gray forever
                let gc_func = func_ptr.as_mut_ref();
                gc_func.header.make_black();

                let upvalues = gc_func.data.upvalues();
                // Mark upvalues
                for upval_ptr in upvalues {
                    let header = &mut upval_ptr.as_mut_ref().header;
                    if header.is_white() {
                        header.make_gray();
                        self.gray.push(upval_ptr.clone().into());
                    }
                }

                // Mark all constants in the chunk and nested chunks (like Lua 5.5's traverseproto)
                if let Some(chunk) = gc_func.data.chunk() {
                    self.mark_chunk_constants(&chunk);
                    return 1
                        + upvalues.len() as isize
                        + chunk.constants.len() as isize
                        + chunk.child_protos.len() as isize;
                } else {
                    return 1 + upvalues.len() as isize;
                }
            }
            GcObjectPtr::Upvalue(upval_ptr) => {
                // Port of Lua 5.5's reallymarkobject for LUA_VUPVAL:
                // if (upisopen(uv))
                //   set2gray(uv);  /* open upvalues are kept gray */
                // else
                //   set2black(uv);  /* closed upvalues are visited here */
                // markvalue(g, uv->v.p);  /* mark its content */
                let gc_upval = upval_ptr.as_mut_ref();

                if gc_upval.data.is_open() {
                    // Open upvalue: keep gray (value on stack may change)
                    gc_upval.header.make_gray();
                    // Note: value is on stack, will be marked when stack is traversed
                    // But we should mark it here too for safety
                    // Get stack index and mark that slot
                    // Since we don't have access to LuaState here, we rely on
                    // the stack being marked separately as a root
                } else {
                    // Closed upvalue: mark black and mark its value
                    gc_upval.header.make_black();
                    if let Some(val) = gc_upval.data.get_closed_value() {
                        self.mark_value(&val);
                    }
                }

                return 1;
            }
            GcObjectPtr::String(string_ptr) => {
                let s = string_ptr.as_mut_ref();
                s.header.make_black();
                return 1;
            }
            GcObjectPtr::Binary(binary_ptr) => {
                let b = binary_ptr.as_mut_ref();
                b.header.make_black();
                return 1;
            }
            GcObjectPtr::Userdata(userdata_ptr) => {
                // Userdata: mark the userdata itself and its metatable if any
                let gc_ud = userdata_ptr.as_mut_ref();
                gc_ud.header.make_black();
                if let Some(metatable) = gc_ud.data.get_metatable() {
                    // Mark metatable if exists (it's a LuaValue, could be table)
                    self.mark_value(&metatable);
                }

                return 1;
            }
            GcObjectPtr::Thread(thread_ptr) => {
                // Thread: mark all stack values and open upvalues
                let gc_thread = thread_ptr.as_mut_ref();
                gc_thread.header.make_black();
                let l = &gc_thread.data;
                let stack_top = l.stack_top;
                for i in 0..stack_top {
                    let value = &l.stack[i];
                    self.mark_value(value);
                }

                // Mark all open upvalues
                for upval_ptr in l.open_upvalues() {
                    let gc_upvalue = upval_ptr.as_mut_ref();
                    if gc_upvalue.header.is_white() {
                        gc_upvalue.header.make_gray();
                        self.gray.push(upval_ptr.clone().into());
                    }
                }

                return 1 + stack_top as isize;
            }
        }
    }

    /// Atomic phase (like atomic in Lua 5.5)
    fn atomic(&mut self, roots: &[LuaValue]) {
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
            self.mark_value(value);
        }

        // Propagate all marks (empty the gray list)
        while !self.gray.is_empty() {
            self.propagate_mark();
        }

        // NOTE: remarkupvals should be called here by VM
        // This marks values in open upvalues for non-marked threads
        // Since we don't have thread list access here, VM must call
        // mark_open_upvalues before calling atomic

        // Process grayagain list (objects that were blackened but then mutated)
        // Moving them to gray list effectively
        let grayagain = std::mem::take(&mut self.grayagain);
        for gc_id in grayagain {
            let size = self.mark_one(gc_id);
            // Update gc_marked for objects processed from grayagain
            self.gc_marked += size;
        }

        // Propagate again to handle anything pushed by grayagain processing
        while !self.gray.is_empty() {
            self.propagate_mark();
        }

        // Port of Lua 5.5's atomic() phase for weak tables
        // At this point, all strongly accessible objects are marked.

        // First convergence of ephemeron tables
        self.converge_ephemerons();

        // Clear weak values (first pass)
        self.clear_by_values();

        // Save original lists before finalization (Lua 5.5: origweak = g->weak; origall = g->allweak;)
        let origweak = self.weak.clone();
        let origall = self.allweak.clone();

        // TODO: Handle finalizers (separatetobefnz, markbeingfnz)
        // For now we skip finalization

        // Second convergence after potential resurrection
        self.converge_ephemerons();

        // Clear weak keys from ephemeron and allweak tables
        self.clear_by_keys();

        // Clear weak values (second pass, only for resurrected objects)
        // Lua 5.5: clearbyvalues(g, g->weak, origweak); clearbyvalues(g, g->allweak, origall);
        // This clears new entries added after finalization
        // Since we skip finalization, this should be a no-op, but keep it for correctness
        self.clear_by_values_range(&origweak, &origall);

        // Flip white color for next cycle
        // After this, objects with old white are considered dead
        self.current_white ^= 1;
    }

    /// Enter sweep phase (like entersweep in Lua 5.5)
    pub fn enter_sweep(&mut self, _pool: &mut ObjectAllocator) {
        self.gc_state = GcState::SwpAllGc;
        self.sweep_index = 0; // Reset sweep position

        // No longer need to cache object IDs - we'll iterate through pool directly
        // This avoids the dangling pointer problem with swap_remove
    }

    /// Sweep step - collect dead objects (like sweepstep in Lua 5.5)
    /// Returns true if sweep is complete (no more objects to sweep)
    ///
    /// Port of Lua 5.5's sweepstep: maintains sweep position (sweepgc pointer)
    /// to avoid re-scanning already-swept objects
    ///
    /// CRITICAL: Lua 5.5's sweeplist does NOT handle finalization!
    /// Objects with finalizers are in a separate 'finobj' list and handled by separatetobefnz.
    /// Here we should ONLY:
    /// 1. Free dead objects (isdeadm check)
    /// 2. Reset surviving objects to current white
    fn sweep_step(&mut self, pool: &mut ObjectAllocator, fast: bool) -> bool {
        let max_sweep = if fast { usize::MAX } else { 100 };
        let mut count = 0;
        let other_white = 1 - self.current_white;

        // Sweep forward through pool, handling swap_remove correctly
        while self.sweep_index < self.gc_pool.len() && count < max_sweep {
            let current_idx = self.sweep_index;

            // Get object info
            let owner = &self.gc_pool.get(current_idx).unwrap();
            let gc_ptr = owner.as_gc_ptr();
            let header = owner.header();

            // Check if object is dead (not fixed and wrong white)
            if !header.is_fixed() && header.is_dead(other_white) {
                // Dead object - remove it
                // Save size BEFORE removing (after free, header is invalid!)
                let size = header.size as usize;

                // For strings, remove from interner first
                if let GcObjectPtr::String(str_ptr) = gc_ptr {
                    pool.remove_str(str_ptr);
                }

                // Free the object (swap_remove: moves last to current_idx)
                self.gc_pool.free(gc_ptr);

                if size > 0 {
                    self.gc_debt += size as isize;
                    self.stats.bytes_freed += size;
                    self.stats.objects_collected += 1;
                }

                // DON'T increment sweep_index - the object from end was moved here
                // Next iteration will check that moved object
            } else if !header.is_fixed() {
                // Lua 5.5 sweeplist: Surviving objects reset to current white!
                // curr->marked = cast_byte((marked & ~maskgcbits) | white | G_NEW);
                if let Some(header_mut) = gc_ptr.header_mut() {
                    header_mut.make_white(self.current_white);
                    header_mut.set_age(G_NEW);
                }
                self.sweep_index += 1; // Move to next
            } else {
                // Fixed object, skip
                self.sweep_index += 1;
            }

            count += 1;
        }

        // Return true if we've swept through the entire pool
        self.sweep_index >= self.gc_pool.len()
    }

    pub fn set_pause(&mut self) {
        // Lua 5.5 lgc.c setpause:
        // l_mem threshold = applygcparam(g, PAUSE, g->GCmarked);
        // l_mem debt = threshold - gettotalbytes(g);
        // if (debt < 0) debt = 0;
        // luaE_setdebt(g, debt);
        let threshold = self.apply_param(PAUSE, self.gc_marked);
        let real_bytes = self.total_bytes - self.gc_debt;
        let mut debt = threshold - real_bytes;

        if debt < 0 {
            debt = 0;
        }
        self.set_debt(debt);

        // With IndexMap, no need to compact - it has no empty slots!
        // shrink_to_fit() only reduces memory overhead, doesn't affect iteration
        self.gc_pool.shrink_to_fit();
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
        pool: &mut ObjectAllocator,
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
    pub fn full_generation(&mut self, roots: &[LuaValue], pool: &mut ObjectAllocator) {
        // If we're in Pause state, we need to do the state transition ourselves
        if self.gc_state == GcState::Pause {
            // CRITICAL: Call restart_collection WHILE STILL IN PAUSE STATE!
            // Lua 5.5's singlestep calls restartcollection() while gcstate==GCSpause,
            // THEN sets gcstate=GCSpropagate.
            self.restart_collection(roots);

            // NOW transition to Propagate state (like Lua 5.5's singlestep)
            self.gc_state = GcState::Propagate;
        }

        // Step 2: Propagate gray list (weak tables will be added to grayagain)
        while !self.gray.is_empty() {
            self.propagate_mark();
        }

        // Step 3: Call atomic phase (like youngcollection does)
        // atomic() will process grayagain, converge ephemerons, and clear weak tables
        self.atomic(roots);

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
    fn young_collection(&mut self, roots: &[LuaValue], pool: &mut ObjectAllocator) {
        self.stats.minor_collections += 1;

        // Start collection: mark roots
        self.restart_collection(roots);
        self.gc_state = GcState::Propagate;

        // Propagate all marks
        while !self.gray.is_empty() {
            self.propagate_mark();
        }

        // CRITICAL: Call atomic phase!
        // This flips current_white so sweep can identify dead objects
        self.atomic(roots);

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
    pub fn barrier(&mut self, o_ptr: GcObjectPtr, v_ptr: GcObjectPtr) {
        // Check if 'o' is black and 'v' is white
        let (o_black, o_old) = if let Some(o) = o_ptr.header() {
            (o.is_black(), o.is_old())
        } else {
            return;
        };
        if !o_black {
            return;
        }

        let v_white = if let Some(v) = v_ptr.header() {
            v.is_white()
        } else {
            return;
        };

        if !v_white {
            return;
        }

        // Must keep invariant during mark phase
        if self.gc_state.keep_invariant() {
            // Mark 'v' immediately to restore invariant
            self.mark_object(v_ptr);

            // Generational invariant: if 'o' is old, make 'v' OLD0
            if o_old {
                if let Some(header) = v_ptr.header_mut() {
                    header.make_old0();
                }
            }
        } else if self.gc_state.is_sweep_phase() {
            // In incremental sweep: make 'o' white to avoid repeated barriers
            if self.gc_kind != GcKind::GenMinor {
                if let Some(header) = o_ptr.header_mut() {
                    header.make_white(self.current_white);
                }
            }
        }
    }

    /// Backward barrier (luaC_barrierback_)
    /// Called when a black object 'o' is modified to point to white object
    /// Instead of marking the white object, we mark 'o' as gray again
    /// Used for tables and other objects that may have many modifications
    pub fn barrier_back(&mut self, o_ptr: GcObjectPtr) {
        let (is_black, age) = if let Some(o) = o_ptr.header() {
            (o.is_black(), o.age())
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
            if let Some(header) = o_ptr.header_mut() {
                header.make_gray();
            }
        } else {
            // In propagate/atomic phase: link into grayagain for re-traversal in current cycle
            // In pause/sweep phase: link into gray for next cycle
            let target_list = if self.gc_state == GcState::Pause || self.gc_state.is_sweep_phase() {
                &mut self.gray
            } else {
                &mut self.grayagain
            };

            if !target_list.contains(&o_ptr) {
                target_list.push(o_ptr);
            }

            if let Some(header) = o_ptr.header_mut() {
                header.make_gray();
            }
        }

        // If old in generational mode: mark as TOUCHED1
        if age >= G_OLD0 {
            if let Some(header) = o_ptr.header_mut() {
                header.make_touched1();
            }
        }
    }

    /// Mark an object (helper for barrier)
    fn mark_object(&mut self, gc_ptr: GcObjectPtr) {
        if let Some(header) = gc_ptr.header_mut() {
            // Only need to mark if it is white
            if header.is_white() {
                match gc_ptr.kind() {
                    GcObjectKind::String | GcObjectKind::Binary => {
                        header.make_black(); // Leaves become black immediately
                    }
                    _ => {
                        header.make_gray(); // Others become gray
                        self.gray.push(gc_ptr);
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
