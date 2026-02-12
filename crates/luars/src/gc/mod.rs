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

use std::collections::HashSet;

use crate::{
    LuaResult, LuaTable,
    lua_value::{Chunk, LuaValue},
    lua_vm::{LuaError, LuaState, SafeOption, TmKind},
};
pub use gc_kind::*;
pub use gc_object::*;
pub use object_allocator::*;
pub use string_interner::*;

// GC Parameters (from lua.h)
pub const MINORMUL: usize = 0; // Minor collection multiplier
pub const MAJORMINOR: usize = 1; // Shift from major to minor
pub const MINORMAJOR: usize = 2; // Shift from minor to major

pub const PAUSE: usize = 3; // Pause between GC cycles (default 200%)
pub const STEPMUL: usize = 4; // GC speed multiplier (default 200)
pub const STEPSIZE: usize = 5; // Step size in KB (default 13KB)

pub const GCPARAM_COUNT: usize = 6;

// Default GC parameters (from Lua 5.5 lgc.h)
// MUST match Lua 5.5 exactly for debugging consistency
const DEFAULT_PAUSE: i32 = 250; // 250% (LUAI_GCPAUSE in lgc.h)
const DEFAULT_STEPMUL: i32 = 200; // 200% (LUAI_GCMUL in lgc.h)
const DEFAULT_STEPSIZE: i32 = 200 * std::mem::size_of::<LuaTable>() as i32; // ~13KB
const DEFAULT_MINORMUL: i32 = 200; // 200% - balanced threshold for minor GC
const DEFAULT_MINORMAJOR: i32 = 70; // 70%
const DEFAULT_MAJORMINOR: i32 = 50; // 50%

const GCSWEEPMAX: isize = 20; // Max steps per sweep call

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
    // === GC object lists by age (matching Lua 5.5's design) ===
    // Lua 5.5 uses a single linked list with pointers to split generations.
    // We use separate GcLists for each generation for O(1) ownership transfer.
    //
    // Object lifecycle in generational GC:
    // 1. New objects created → allgc (G_NEW, nursery)
    // 2. Survived one collection → survival (G_SURVIVAL)
    // 3. Survived two collections → old (G_OLD1, G_OLD)
    //
    // During youngcollection:
    // - Dead objects in allgc/survival are freed
    // - Surviving allgc objects move to survival
    // - Surviving survival objects move to old
    /// G_NEW objects (nursery) - newly created objects
    allgc: GcList,

    /// G_SURVIVAL objects - survived one minor collection
    survival: GcList,

    /// G_OLD1 objects - survived two collections, need marking in next young collection
    /// This is a performance optimization: instead of scanning entire old list in mark_old,
    /// we only scan this small list of recently promoted objects.
    old1: GcList,

    /// G_OLD, G_TOUCHED1, G_TOUCHED2 objects - old generation (stable)
    old: GcList,

    /// Objects not to be collected (like fixedgc in Lua 5.5)
    fixed_list: GcList,
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

    /// Objects pending __gc finalization (Lua 5.5: g->tobefnz)
    ///
    /// We keep a Vec instead of a linked list; objects stay in the main pool.
    /// Items are moved into VM actions only in CallFin state (after sweep),
    /// matching Lua 5.5 timing.
    tobefnz: Vec<GcObjectPtr>,

    // === GC parameters (from gcparams[LUA_GCPN]) ===
    // Stored as compressed floating-point bytes (like Lua 5.5)
    // Use code_param() to encode, apply_param() to apply
    pub gc_params: [u8; GCPARAM_COUNT],

    // === Gray lists (for marking) ===
    /// Regular gray objects waiting to be visited
    gray: Vec<GcObjectPtr>,

    /// Objects to be revisited at atomic phase
    grayagain: Vec<GcObjectPtr>,

    // === Weak table lists (Port of Lua 5.5) ===
    /// Weak value tables (only values are weak)
    weak: Vec<TablePtr>,

    /// Ephemeron tables (keys are weak)
    ephemeron: Vec<TablePtr>,

    /// Fully weak tables (both keys and values are weak)
    allweak: Vec<TablePtr>,

    /// Threads with open upvalues
    twups: Vec<ThreadPtr>,

    /// Finalizers called during GC
    finobj: Vec<GcObjectPtr>,

    // === Sweep state ===
    /// Current position in sweep (like Lua 5.5's sweepgc pointer)
    /// This ensures we don't re-scan the same objects
    sweepgc: SweepGc,

    // === Statistics ===
    pub stats: GcStats,

    pub tm_gc: LuaValue,

    pub tm_mode: LuaValue,

    max_memory_limit: isize,

    tmp_max_memory_limit: Option<isize>,

    gc_error_msg: Option<String>,

    gc_memory_check: bool,
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
    pub fn new(option: SafeOption) -> Self {
        let mut gc = GC {
            allgc: GcList::new(),
            survival: GcList::new(),
            old1: GcList::new(),
            old: GcList::new(),
            fixed_list: GcList::new(),
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
            tobefnz: Vec::new(),
            gc_params: [0; GCPARAM_COUNT], // Default to 100%
            gray: Vec::with_capacity(128),
            grayagain: Vec::with_capacity(64),
            weak: Vec::new(),
            ephemeron: Vec::new(),
            allweak: Vec::new(),
            twups: Vec::new(),
            finobj: Vec::new(),
            sweepgc: SweepGc::AllGc(0),
            stats: GcStats::default(),
            tm_gc: LuaValue::nil(),
            tm_mode: LuaValue::nil(),
            max_memory_limit: option.max_memory_limit as isize,
            tmp_max_memory_limit: None,
            gc_error_msg: None,
            gc_memory_check: true,
        };

        gc.gc_params[PAUSE] = code_param(DEFAULT_PAUSE as u32);
        gc.gc_params[STEPMUL] = code_param(DEFAULT_STEPMUL as u32);
        gc.gc_params[STEPSIZE] = code_param(DEFAULT_STEPSIZE as u32);
        gc.gc_params[MINORMUL] = code_param(DEFAULT_MINORMUL as u32);
        gc.gc_params[MINORMAJOR] = code_param(DEFAULT_MINORMAJOR as u32);
        gc.gc_params[MAJORMINOR] = code_param(DEFAULT_MAJORMINOR as u32);

        gc
    }

    /// Helper to remove a dead string from the string intern map
    /// This is needed because object_allocator is in LuaVM, accessed via LuaState
    fn remove_dead_string_from_intern(l: &mut LuaState, str_ptr: StringPtr) {
        l.remove_dead_string(str_ptr);
    }

    /// Change to incremental mode (like minor2inc in Lua 5.5)
    ///
    /// Port of Lua 5.5 lgc.c minor2inc:
    /// ```c
    /// static void minor2inc (lua_State *L, global_State *g, lu_byte kind) {
    ///   g->GCmajorminor = g->GCmarked;
    ///   g->gckind = kind;
    ///   g->reallyold = g->old1 = g->survival = NULL;
    ///   g->finobjrold = g->finobjold1 = g->finobjsur = NULL;
    ///   entersweep(L);
    ///   luaE_setdebt(g, applygcparam(g, STEPSIZE, 100));
    /// }
    /// ```
    ///
    /// In Lua 5.5, all objects are in a single linked list (allgc) with pointer
    /// markers for generation boundaries. Setting those markers to NULL removes
    /// the boundaries. In our implementation, we use separate GcLists per
    /// generation, so we must merge them back into allgc.
    pub fn change_to_incremental_mode(&mut self, l: &mut LuaState) {
        if self.gc_kind == GcKind::Inc {
            return; // Already in incremental mode
        }

        // Save number of live bytes
        self.gc_majorminor = self.gc_marked;

        // Switch mode
        self.gc_kind = GcKind::Inc;

        // Merge all generation lists into allgc
        // (equivalent to Lua 5.5's clearing of survival/old1/reallyold pointers)
        let survival_objects = self.survival.take_all();
        self.allgc.add_all(survival_objects);

        let old1_objects = self.old1.take_all();
        self.allgc.add_all(old1_objects);

        let old_objects = self.old.take_all();
        self.allgc.add_all(old_objects);

        // Enter sweep phase (like Lua 5.5's entersweep)
        self.enter_sweep(l);

        // Set debt for next step
        let stepsize = self.apply_param(STEPSIZE, 100);
        self.set_debt(stepsize);
    }

    /// Enter generational mode from incremental mode.
    ///
    /// Port of Lua 5.5 lgc.c entergen:
    /// ```c
    /// static void entergen (lua_State *L, global_State *g) {
    ///   luaC_runtilstate(L, GCSpause, 1);
    ///   luaC_runtilstate(L, GCSpropagate, 1);
    ///   atomic(L);
    ///   atomic2gen(L, g);
    ///   setminordebt(g);
    /// }
    /// ```
    pub fn enter_gen(&mut self, l: &mut LuaState) {
        // Must be in incremental mode (Lua 5.5 asserts gckind == KGC_INC)
        debug_assert!(self.gc_kind == GcKind::Inc);

        // Complete any in-progress cycle
        self.run_until_state(l, GcState::Pause);

        // Start a fresh cycle: Pause → restart_collection → Propagate
        self.run_until_state(l, GcState::Propagate);

        // Run atomic phase (marks all, propagates, converges ephemerons, etc.)
        self.atomic(l);

        // Transition to generational mode: sweep all to old, set up gen structures
        self.atomic2gen(l);

        // Set debt for next minor collection
        self.set_minor_debt();
    }

    /// Change GC mode (like luaC_changemode in Lua 5.5)
    ///
    /// Port of Lua 5.5 lgc.c:
    /// ```c
    /// void luaC_changemode (lua_State *L, int newmode) {
    ///   global_State *g = G(L);
    ///   if (g->gckind == KGC_GENMAJOR)
    ///     g->gckind = KGC_INC;
    ///   if (newmode != g->gckind) {
    ///     if (newmode == KGC_INC)
    ///       minor2inc(L, g, KGC_INC);
    ///     else {
    ///       lua_assert(newmode == KGC_GENMINOR);
    ///       entergen(L, g);
    ///     }
    ///   }
    /// }
    /// ```
    pub fn change_mode(&mut self, l: &mut LuaState, new_mode: GcKind) {
        // GenMajor is really incremental under the hood
        if self.gc_kind == GcKind::GenMajor {
            self.gc_kind = GcKind::Inc;
        }

        if new_mode == GcKind::Inc {
            self.change_to_incremental_mode(l);
        } else {
            debug_assert!(new_mode == GcKind::GenMinor);
            if self.gc_kind != GcKind::GenMinor {
                self.enter_gen(l);
            }
        }
    }

    pub fn trace_object(&mut self, gc_object_owner: GcObjectOwner) -> LuaResult<()> {
        let size = gc_object_owner.size_of_data();

        // SAFETY CHECK: Prevent out-of-memory by limiting total allocation to MAX_MEMORY_LIMIT
        let total_bytes = self.get_total_bytes();
        let limit_bytes = self.get_limit_bytes();
        if self.gc_memory_check && (total_bytes + size as isize > limit_bytes) {
            // For simple test, later will return an error instead of panic
            self.gc_error_msg = Some(format!(
                "Memory limit exceeded: {} bytes allocated, attempting to allocate {} more bytes (limit: {} bytes)",
                total_bytes, size, limit_bytes,
            ));
            return Err(LuaError::OutOfMemory);
        }

        let age = gc_object_owner.header().age();

        // Add to appropriate generation list based on age
        // New objects (G_NEW) go to allgc (nursery)
        // This matches Lua 5.5's design where new objects are prepended to allgc
        match age {
            G_NEW => self.allgc.add(gc_object_owner),
            G_SURVIVAL => self.survival.add(gc_object_owner),
            G_OLD1 => self.old1.add(gc_object_owner),
            _ => self.old.add(gc_object_owner), // G_OLD0, G_OLD, G_TOUCHED*
        }

        self.track_size(size);

        Ok(())
    }

    /// Track a new object allocation (like luaC_newobj in Lua)
    /// This increments debt - when debt becomes positive, GC should run
    ///
    #[inline]
    fn track_size(&mut self, size: usize) {
        let size_signed = size as isize;

        // Lua 5.5 lmem.c luaM_malloc_:
        self.gc_debt -= size_signed; // 分配减少debt
        self.stats.bytes_allocated += size;
    }

    pub fn fixed(&mut self, gc_ptr: GcObjectPtr) {
        // Find which list the object is in and remove it
        let gc_owner = if let Some(header) = gc_ptr.header() {
            match header.age() {
                G_NEW => self.allgc.remove(gc_ptr),
                G_SURVIVAL => self.survival.remove(gc_ptr),
                G_OLD1 => self.old1.remove(gc_ptr),
                _ => self.old.remove(gc_ptr),
            }
        } else {
            // Fallback: try each list
            if self.allgc.contains(gc_ptr) {
                self.allgc.remove(gc_ptr)
            } else if self.survival.contains(gc_ptr) {
                self.survival.remove(gc_ptr)
            } else if self.old1.contains(gc_ptr) {
                self.old1.remove(gc_ptr)
            } else {
                self.old.remove(gc_ptr)
            }
        };

        self.fixed_list.add(gc_owner);

        if let Some(header) = gc_ptr.header_mut() {
            header.set_age(G_OLD);
            header.make_gray(); // Gray forever, like Lua 5.5
        }
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
        let real_bytes = self.get_total_bytes();

        // Avoid overflow in total_bytes
        if debt > MAX_LMEM - real_bytes {
            debt = MAX_LMEM - real_bytes;
        }

        // Maintain invariant: total_bytes = real_bytes + debt
        self.total_bytes = real_bytes + debt;
        self.gc_debt = debt;
    }

    fn get_total_bytes(&self) -> isize {
        self.total_bytes - self.gc_debt
    }

    /// Release a GC object by dropping it.
    #[inline]
    fn release_object(&mut self, obj: GcObjectOwner) {
        drop(obj);
    }

    /// Static version of release_object for use in closures that can't borrow `self`.
    #[inline]
    fn release_object_static(obj: GcObjectOwner) {
        drop(obj);
    }

    // Debug accessors for list lengths
    pub fn allgc_len(&self) -> usize {
        self.allgc.len()
    }

    pub fn survival_len(&self) -> usize {
        self.survival.len()
    }

    pub fn old_len(&self) -> usize {
        self.old.len()
    }

    pub fn fixed_len(&self) -> usize {
        self.fixed_list.len()
    }

    fn get_limit_bytes(&self) -> isize {
        if let Some(tmp_limit) = self.tmp_max_memory_limit {
            tmp_limit
        } else {
            self.max_memory_limit
        }
    }

    /// set new additional temporary memory limit
    pub fn set_temporary_memory_limit(&mut self, limit: isize) {
        let current_total_bytes = self.get_total_bytes();
        self.tmp_max_memory_limit = Some(current_total_bytes.saturating_add(limit));
    }

    pub fn clear_temporary_memory_limit(&mut self) {
        self.tmp_max_memory_limit = None;
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
    fn is_cleared(&mut self, l: &mut LuaState, gc_ptr: GcObjectPtr) -> bool {
        match gc_ptr.kind() {
            GcObjectKind::String | GcObjectKind::Binary => {
                self.mark_object(l, gc_ptr);
                false
            }
            _ => self.is_white(gc_ptr),
        }
    }

    /// Check if an object needs finalization (__gc metamethod)
    /// Only tables, userdata, and threads can have __gc
    fn needs_finalization(&self, gc_ptr: GcObjectPtr) -> bool {
        let metatable = match gc_ptr {
            GcObjectPtr::Table(table_ptr) => table_ptr.as_ref().data.get_metatable(),
            GcObjectPtr::Userdata(ud_ptr) => ud_ptr.as_ref().data.get_metatable(),
            _ => {
                return false;
            }
        };

        if let Some(metatable) = metatable {
            let gc_key = self.tm_gc;
            if let Some(mt_table) = metatable.as_table() {
                let gc_field = mt_table.raw_get(&gc_key);
                if let Some(gc_field) = gc_field {
                    let result = !gc_field.is_nil();
                    return result;
                }
            }
        }

        false
    }

    /// Register an object as finalizable if its metatable has a non-nil __gc.
    ///
    /// This mirrors Lua 5.5's luaC_checkfinalizer: objects are put into the
    /// 'finobj' list when __gc is set. We model this by setting FINALIZEDBIT
    /// and later, during atomic, moving unreachable ones into `tobefnz`.
    pub fn check_finalizer(&mut self, value: &LuaValue) {
        let Some(gc_ptr) = value.as_gc_ptr() else {
            return;
        };

        match gc_ptr {
            GcObjectPtr::Table(_) | GcObjectPtr::Userdata(_) | GcObjectPtr::Thread(_) => {}
            _ => return,
        }

        let Some(header) = gc_ptr.header_mut() else {
            return;
        };

        if header.to_finalize() {
            return;
        };

        let needs = self.needs_finalization(gc_ptr);

        if needs {
            header.set_finalized();
            self.finobj.push(gc_ptr);
        }
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
    fn converge_ephemerons(&mut self, l: &mut LuaState) {
        let mut changed;
        let mut dir = false;

        loop {
            let ephemeron_list = std::mem::take(&mut self.ephemeron);
            self.ephemeron.clear();
            changed = false;

            for table_ptr in ephemeron_list {
                table_ptr.as_mut_ref().header.make_black();

                // Use traverse_ephemeron_atomic for convergence
                let marked = self.traverse_ephemeron_atomic(l, table_ptr, dir);
                if marked {
                    self.propagate_all(l);
                    changed = true;
                }
            }

            dir = !dir;
            if !changed {
                break;
            }
        }
    }

    /// Traverse ephemeron in atomic phase - returns true if any value was marked
    fn traverse_ephemeron_atomic(
        &mut self,
        l: &mut LuaState,
        table_ptr: TablePtr,
        inv: bool,
    ) -> bool {
        let entries = table_ptr.as_ref().data.iter_all();

        let mut marked_any = false;
        let mut has_white_keys = false;
        let mut has_white_white = false;

        let mut entry_list: Vec<_> = entries.into_iter().collect();
        if inv {
            entry_list.reverse();
        }

        for (k, v) in &entry_list {
            let key_ptr = k.as_gc_ptr();
            let val_ptr = v.as_gc_ptr();

            let key_is_cleared = key_ptr.map_or(false, |ptr| self.is_cleared(l, ptr));
            let val_is_white = val_ptr.map_or(false, |ptr| self.is_white(ptr));

            if key_is_cleared {
                has_white_keys = true;
                if val_is_white {
                    has_white_white = true;
                }
            } else if val_is_white {
                self.really_mark_object(l, val_ptr.unwrap());
                marked_any = true;
            }
        }

        if has_white_white {
            self.ephemeron.push(table_ptr);
        } else if has_white_keys {
            self.allweak.push(table_ptr);
        }

        marked_any
    }

    /// Port of Lua 5.5's clearbykeys
    /// Clear entries with unmarked keys from ephemeron and fully weak tables
    fn clear_by_keys(&mut self, l: &mut LuaState) {
        // Clear ephemeron tables
        let ephemeron_list = self.ephemeron.clone();
        for table_ptr in ephemeron_list {
            self.clear_table_by_keys(l, table_ptr);
        }

        // Clear fully weak tables
        let allweak_list = self.allweak.clone();
        for table_ptr in allweak_list {
            self.clear_table_by_keys(l, table_ptr);
        }
    }

    /// Clear entries with unmarked keys from a single table
    fn clear_table_by_keys(&mut self, l: &mut LuaState, table_ptr: TablePtr) -> usize {
        // CRITICAL FIX: Collect all keys first to avoid holding reference during is_cleared()
        // This prevents use-after-free and borrowing issues
        let entries = table_ptr.as_ref().data.iter_keys();

        let mut keys_to_remove = Vec::new();

        for key in entries {
            if let Some(key_ptr) = key.as_gc_ptr() {
                if self.is_cleared(l, key_ptr) {
                    keys_to_remove.push(key);
                }
            }
        }

        let count = keys_to_remove.len();
        // Remove entries with dead keys
        let table = &mut table_ptr.as_mut_ref().data;
        for key in keys_to_remove {
            table.raw_set(&key, LuaValue::nil());
        }
        count
    }

    /// Port of Lua 5.5's clearbyvalues
    /// Clear entries with unmarked values from weak value tables
    fn clear_by_values(&mut self, l: &mut LuaState) {
        let weak_list = self.weak.clone();
        for table_ptr in weak_list {
            self.clear_table_by_values(l, table_ptr);
        }

        let allweak_list = self.allweak.clone();
        for table_ptr in allweak_list {
            self.clear_table_by_values(l, table_ptr);
        }
    }

    /// Clear entries with unmarked values from a single table
    fn clear_table_by_values(&mut self, l: &mut LuaState, table_ptr: TablePtr) {
        let entries = table_ptr.as_ref().data.iter_all();

        let mut keys_to_remove = Vec::new();

        for (k, value) in entries {
            if let Some(val_ptr) = value.as_gc_ptr() {
                if self.is_cleared(l, val_ptr) {
                    keys_to_remove.push(k);
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
    pub fn step(&mut self, l: &mut LuaState) {
        // Lua 5.5 luaC_step:
        // if (!gcrunning(g)) {
        //   if (g->gcstp & GCSTPUSR) luaE_setdebt(g, 20000);
        // } else { ... }

        // Check if GC is stopped by user (unless forced)
        if self.gc_stopped {
            // Lua 5.5: set reasonable debt to avoid being called at every check
            self.set_debt(20000);
            return;
        }

        // BUG FIX: Prevent GC reentrancy during finalization or single_step.
        // In Lua 5.5, gcrunning(g) checks (g->gcstp == 0), which includes GCSTPGC.
        // Without this check, check_gc() during a finalizer's pcall can trigger
        // recursive GC steps → recursive young_collection → stack overflow or
        // state corruption → hangs and use-after-free.
        if self.gc_stopem {
            self.set_debt(20000);
            return;
        }

        // Dispatch based on GC mode (like Lua 5.5 luaC_step)
        match self.gc_kind {
            GcKind::Inc | GcKind::GenMajor => self.inc_step(l),
            GcKind::GenMinor => {
                self.young_collection(l);
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
    fn inc_step(&mut self, l: &mut LuaState) {
        // l_mem stepsize = applygcparam(g, STEPSIZE, 100);
        let stepsize = self.apply_param(STEPSIZE, 100);

        // l_mem work2do = applygcparam(g, STEPMUL, stepsize / cast_int(sizeof(void*)));
        let ptr_size = std::mem::size_of::<*const ()>() as isize;
        let mut work2do = self.apply_param(STEPMUL, stepsize / ptr_size);
        // int fast = (work2do == 0);
        let fast = work2do == 0;

        // Repeat until enough work is done (like Lua 5.5's do-while loop)
        loop {
            let stres = self.single_step(l, fast);

            match stres {
                StepResult::Step2Minor => {
                    // Returned to minor collections
                    return;
                }
                StepResult::Step2Pause => {
                    // End of cycle (step2pause in Lua)
                    break;
                }
                StepResult::AtomicStep => {
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
    fn single_step(&mut self, l: &mut LuaState, fast: bool) -> StepResult {
        self.gc_stopem = true;

        let result = match self.gc_state {
            GcState::Pause => {
                self.restart_collection(l);
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
                    let work = self.propagate_mark(l);
                    StepResult::Work(work)
                }
            }
            GcState::EnterAtomic => {
                self.atomic(l);
                if self.check_major_minor(l) {
                    StepResult::Step2Minor
                } else {
                    self.enter_sweep(l);
                    StepResult::AtomicStep
                }
            }
            GcState::SwpAllGc => {
                self.sweep_step(l, GcState::SwpFinObj, SweepGc::FinObj(0), fast);
                StepResult::Work(GCSWEEPMAX) // GCSWEEPMAX equivalent
            }
            GcState::SwpFinObj => {
                self.sweep_step(l, GcState::SwpToBeFnz, SweepGc::ToBeFnz(0), fast);
                StepResult::Work(GCSWEEPMAX)
            }
            GcState::SwpToBeFnz => {
                self.sweep_step(l, GcState::SwpEnd, SweepGc::Done, fast);
                StepResult::Work(GCSWEEPMAX)
            }
            GcState::SwpEnd => {
                self.gc_state = GcState::CallFin;
                StepResult::Work(GCSWEEPMAX)
            }
            GcState::CallFin => {
                // Lua 5.5: GCScallfin calls pending finalizers from 'tobefnz'.
                // Each step calls ONE finalizer (GCTM)
                if !self.tobefnz.is_empty() && !self.gc_emergency {
                    // Call one finalizer
                    self.call_one_finalizer(l);
                    // Stay in CallFin state to process more finalizers
                    StepResult::Work(GCSWEEPMAX)
                } else {
                    // No more finalizers
                    self.gc_state = GcState::Pause;
                    StepResult::Step2Pause
                }
            }
            GcState::Atomic => {
                // Should not reach here directly
                return StepResult::Work(0);
            }
        };

        self.gc_stopem = false;
        result
    }

    fn clear_gray_lists(&mut self) {
        self.gray.clear();
        self.grayagain.clear();
        self.weak.clear();
        self.ephemeron.clear();
        self.allweak.clear();
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
    ///  Lua 5.5 calls cleargraylists which clears ALL gray lists including weak table lists.
    /// This is safe because restartcollection is only called from GCSpause state,
    /// meaning the previous cycle has completely finished (atomic phase cleared weak tables).
    fn restart_collection(&mut self, l: &mut LuaState) {
        self.stats.collection_count += 1;

        //  Reset sweep_index when starting a new cycle
        // This ensures the next sweep will scan all objects from the beginning
        self.sweepgc = SweepGc::Done;

        // Clear all gray lists (like Lua 5.5's cleargraylists)
        self.clear_gray_lists();

        self.gc_marked = 0;

        // For full/incremental GC, all objects should be in allgc
        // In generational mode, objects are distributed across allgc/survival/old

        // Mark objects
        let main_thread_ptr = l.vm_mut().get_main_thread_ptr();
        self.mark_object(l, main_thread_ptr.into());

        let registry = l.vm_mut().registry;
        // markvalue(g, &g->l_registry);
        self.mark_value(l, &registry);

        // markmt(g);  /* mark global metatables */
        self.mark_mt(l);

        // Mark all threads in twups list (threads with open upvalues)
        // This ensures they are not white when remark_upvalues is called in atomic phase
        for thread_ptr in self.twups.clone() {
            self.mark_object(l, thread_ptr.into());
        }

        // markbeingfnz(g): mark any object pending finalization from previous cycle
        if !self.tobefnz.is_empty() {
            for obj_ptr in self.tobefnz.clone() {
                self.mark_object(l, obj_ptr.into());
            }
        }
    }

    fn mark_mt(&mut self, l: &mut LuaState) {
        for mt in l.vm_mut().get_basic_metatables() {
            if let Some(mt_ptr) = mt.as_gc_ptr() {
                self.mark_object(l, mt_ptr.into());
            }
        }
    }

    fn check_major_minor(&mut self, l: &mut LuaState) -> bool {
        if self.gc_kind == GcKind::GenMinor {
            let num_bytes = self.get_total_bytes();
            let added_bytes = num_bytes - self.gc_majorminor;
            let limit = self.apply_param(MAJORMINOR, added_bytes);
            let to_be_collected = num_bytes - self.gc_marked;

            if to_be_collected >= limit {
                // atomic2gen(L, g);  /* return to generational mode */
                // setminordebt(g);
                self.atomic2gen(l);

                return true;
            }
        }

        self.gc_majorminor = self.gc_marked;
        false
    }

    fn atomic2gen(&mut self, l: &mut LuaState) {
        self.clear_gray_lists();
        self.gc_state = GcState::SwpAllGc;

        let mut to_clear_hash_set = HashSet::new();
        self.sweep2old(l, &mut to_clear_hash_set);

        if !to_clear_hash_set.is_empty() {
            let finobjs = std::mem::take(&mut self.finobj);
            for obj_ptr in finobjs {
                if !to_clear_hash_set.contains(&obj_ptr) {
                    self.finobj.push(obj_ptr);
                }
            }

            // CRITICAL BUG FIX: Don't remove objects from tobefnz even if marked for clearing
            // Objects in tobefnz need to have their finalizers called. If they're in
            // to_clear_hash_set (white), we make them old+black to prevent collection.
            // This fixes the issue where finalizers weren't called in generational GC.
            let tobefnz = std::mem::take(&mut self.tobefnz);
            for obj_ptr in tobefnz {
                if to_clear_hash_set.contains(&obj_ptr) {
                    // Object was marked for clearing but needs finalization
                    // Remove from clear set and make it old+black
                    to_clear_hash_set.remove(&obj_ptr);
                    if let Some(header) = obj_ptr.header_mut() {
                        header.set_age(G_OLD);
                        header.make_black();
                    }
                }
                self.tobefnz.push(obj_ptr);
            }

            for gc_ptr in to_clear_hash_set {
                if let GcObjectPtr::String(str_ptr) = gc_ptr {
                    Self::remove_dead_string_from_intern(l, str_ptr);
                }
                // Remove from the appropriate generation list
                if let Some(header) = gc_ptr.header() {
                    match header.age() {
                        G_NEW => {
                            let obj = self.allgc.remove(gc_ptr);
                            self.total_bytes -= obj.size() as isize;
                            self.release_object(obj);
                        }
                        G_SURVIVAL => {
                            let obj = self.survival.remove(gc_ptr);
                            self.total_bytes -= obj.size() as isize;
                            self.release_object(obj);
                        }
                        G_OLD1 => {
                            let obj = self.old1.remove(gc_ptr);
                            self.total_bytes -= obj.size() as isize;
                            self.release_object(obj);
                        }
                        _ => {
                            let obj = self.old.remove(gc_ptr);
                            self.total_bytes -= obj.size() as isize;
                            self.release_object(obj);
                        }
                    }
                } else {
                    // Fallback: try each list
                    let obj = if self.allgc.contains(gc_ptr) {
                        self.allgc.remove(gc_ptr)
                    } else if self.survival.contains(gc_ptr) {
                        self.survival.remove(gc_ptr)
                    } else if self.old1.contains(gc_ptr) {
                        self.old1.remove(gc_ptr)
                    } else if self.old.contains(gc_ptr) {
                        self.old.remove(gc_ptr)
                    } else {
                        continue;
                    };
                    self.total_bytes -= obj.size() as isize;
                    self.release_object(obj);
                }
            }
        }

        // Move all surviving objects from allgc/survival/old1 into old list
        // (In Lua 5.5: g->reallyold = g->old1 = g->survival = g->allgc;
        //  All pointers merge into one list since everything is now G_OLD)
        let allgc_survivors = self.allgc.take_all();
        self.old.add_all(allgc_survivors);
        let survival_survivors = self.survival.take_all();
        self.old.add_all(survival_survivors);
        let old1_survivors = self.old1.take_all();
        self.old.add_all(old1_survivors);

        self.gc_kind = GcKind::GenMinor;
        self.gc_majorminor = self.gc_marked;
        self.gc_marked = 0;

        // After sweep2old + move, allgc, survival and old1 should be empty
        debug_assert!(
            self.allgc.is_empty() && self.survival.is_empty() && self.old1.is_empty(),
            "allgc, survival and old1 should be empty after atomic2gen"
        );

        self.finish_gen_cycle(l);
    }

    /// Finish a young-generation collection.
    /// Port of Lua 5.5's finishgencycle:
    /// ```c
    /// static void finishgencycle (lua_State *L, global_State *g) {
    ///   correctgraylists(g);
    ///   checkSizes(L, g);
    ///   g->gcstate = GCSpropagate;  /* skip restart */
    ///   if (!g->gcemergency && luaD_checkminstack(L))
    ///     callallpendingfinalizers(L);
    /// }
    /// ```
    fn finish_gen_cycle(&mut self, l: &mut LuaState) {
        // 1. Correct gray lists (handle TOUCHED objects)
        self.correct_gray_lists();

        // 2. checkSizes - optional optimization to shrink tables
        // Skip for now, not critical for correctness

        // 3. Set state to Propagate (skip restart in next cycle)
        self.gc_state = GcState::Propagate;

        // 4. Call pending finalizers if not in emergency mode
        if !self.gc_emergency && !self.tobefnz.is_empty() {
            self.call_all_pending_finalizers(l);
        }
    }

    /// Port of Lua 5.5's correctgraylists
    /// Process TOUCHED objects and advance their ages
    // static void correctgraylists (global_State *g) {
    //      GCObject **list = correctgraylist(&g->grayagain);
    //      *list = g->weak; g->weak = NULL;
    //      list = correctgraylist(list);
    //      *list = g->allweak; g->allweak = NULL;
    //      list = correctgraylist(list);
    //      *list = g->ephemeron; g->ephemeron = NULL;
    //      correctgraylist(list);
    // }
    fn correct_gray_lists(&mut self) {
        // Process grayagain list: handle TOUCHED objects
        let mut grayagain_list = std::mem::take(&mut self.grayagain);
        self.correct_gray_list(&mut grayagain_list);
        self.grayagain = grayagain_list;

        let mut weak_list = std::mem::take(&mut self.weak)
            .iter()
            .map(|ptr| GcObjectPtr::Table(*ptr))
            .collect::<Vec<_>>();
        self.weak.clear();
        // Process weak list: handle TOUCHED objects
        self.correct_gray_list(&mut weak_list);
        self.grayagain.extend(weak_list);

        let mut allweak_list = std::mem::take(&mut self.allweak)
            .iter()
            .map(|ptr| GcObjectPtr::Table(*ptr))
            .collect::<Vec<_>>();
        self.allweak.clear();

        // Process allweak list: handle TOUCHED objects
        self.correct_gray_list(&mut allweak_list);
        self.grayagain.extend(allweak_list);

        let mut ephemeron_list = std::mem::take(&mut self.ephemeron)
            .iter()
            .map(|ptr| GcObjectPtr::Table(*ptr))
            .collect::<Vec<_>>();
        self.ephemeron.clear();

        // Process ephemeron list: handle TOUCHED objects
        self.correct_gray_list(&mut ephemeron_list);
        self.grayagain.extend(ephemeron_list);
    }

    // ** Correct a list of gray objects. Return a pointer to the last element
    // ** left on the list, so that we can link another list to the end of
    // ** this one.
    // ** Because this correction is done after sweeping, young objects might
    // ** be turned white and still be in the list. They are only removed.
    // ** 'TOUCHED1' objects are advanced to 'TOUCHED2' and remain on the list;
    // ** Non-white threads also remain on the list. 'TOUCHED2' objects and
    // ** anything else become regular old, are marked black, and are removed
    // ** from the list.
    fn correct_gray_list(&mut self, list: &mut Vec<GcObjectPtr>) {
        let original_list = std::mem::take(list);

        for gc_ptr in original_list {
            if let Some(header) = gc_ptr.header_mut() {
                if header.is_white() {
                    // Object turned white during sweep - remove from list
                    continue;
                }

                let age = header.age();
                if age == G_TOUCHED1 {
                    header.make_black();
                    header.set_age(G_TOUCHED2);
                    // Keep in list for next cycle
                    list.push(gc_ptr);
                } else if gc_ptr.kind() == GcObjectKind::Thread {
                    // Non-white threads remain in list unchanged
                    list.push(gc_ptr);
                } else {
                    if age == G_TOUCHED2 {
                        // Other ages: become old and black, remove from list
                        header.set_age(G_OLD);
                    }
                    header.make_black();
                }
            }
        }
    }

    /// Mark OLD1 objects for young collection (port of Lua 5.5's markold).
    /// In generational mode, OLD1 objects are those that just became old in the
    /// previous cycle. They need to be traversed because they might reference
    /// young objects that need to be marked.
    ///
    /// Port of Lua 5.5 lgc.c:
    /// ```c
    /// static void markold (global_State *g, GCObject *from, GCObject *to) {
    ///   GCObject *p;
    ///   for (p = from; p != to; p = p->next) {
    ///     if (getage(p) == G_OLD1) {
    ///       lua_assert(!iswhite(p));
    ///       setage(p, G_OLD);  /* now they are old */
    ///       if (isblack(p))
    ///         reallymarkobject(g, p);
    ///     }
    ///   }
    /// }
    /// ```
    ///
    /// OPTIMIZATION: OLD1 objects are now in a separate 'old1' list.
    /// We only iterate the old1 list (small) instead of the entire old list.
    /// After processing, objects are moved from old1 to old.
    ///
    /// NOTE: Objects in old1 may not all have G_OLD1 age - barrier_back can
    /// change an OLD1 object to TOUCHED1 while it's still in the old1 list.
    /// In Lua 5.5, markold simply skips non-OLD1 objects. We do the same.
    fn mark_old(&mut self, l: &mut LuaState) {
        // Process OLD1 objects: mark them and move to old list
        // Take all objects from old1 - they will be moved to old after processing
        let old1_objects = self.old1.take_all();
        let mut to_old: Vec<GcObjectOwner> = Vec::new();

        for gc_owner in old1_objects {
            let gc_ptr = gc_owner.as_gc_ptr();
            if let Some(header) = gc_ptr.header_mut() {
                if header.age() == G_OLD1 {
                    // OLD1 → OLD, and re-mark if black
                    debug_assert!(!header.is_white(), "OLD1 object should not be white");
                    header.set_age(G_OLD);
                    if header.is_black() {
                        self.really_mark_object(l, gc_ptr);
                    }
                }
                // else: age was changed (e.g., to TOUCHED1 by barrier_back),
                // just move to old list - it will be handled by correctgraylists
            }
            to_old.push(gc_owner);
        }

        // Move all processed OLD1 objects to old list
        self.old.add_all(to_old);

        // Mark OLD1 objects in finobj list
        let finobj_list = self.finobj.clone();
        for gc_ptr in finobj_list {
            if let Some(header) = gc_ptr.header_mut() {
                if header.age() == G_OLD1 {
                    header.set_age(G_OLD);
                    if header.is_black() {
                        self.really_mark_object(l, gc_ptr);
                    }
                }
            }
        }

        // Mark OLD1 objects in tobefnz list
        let tobefnz_list = self.tobefnz.clone();
        for gc_ptr in tobefnz_list {
            if let Some(header) = gc_ptr.header_mut() {
                if header.age() == G_OLD1 {
                    header.set_age(G_OLD);
                    if header.is_black() {
                        self.really_mark_object(l, gc_ptr);
                    }
                }
            }
        }
    }

    fn mark_value(&mut self, l: &mut LuaState, value: &LuaValue) {
        let Some(gc_ptr) = value.as_gc_ptr() else {
            return;
        };

        self.mark_object(l, gc_ptr);
    }

    /// Mark all constants in a chunk and its nested chunks (like Lua 5.5's traverseproto)
    fn mark_chunk_constants(&mut self, l: &mut LuaState, chunk: &Chunk) -> usize {
        // Mark all constants in this chunk
        for constant in &chunk.constants {
            if let Some(gc_ptr) = constant.as_gc_ptr() {
                self.mark_object(l, gc_ptr);
            }
        }
        let mut count = chunk.constants.len();

        // Recursively mark constants in child protos (nested functions)
        for child_chunk in &chunk.child_protos {
            count += self.mark_chunk_constants(l, child_chunk);
        }

        count
    }

    // ============ Weak Table Traversal Functions (Port of Lua 5.5) ============
    fn traverse_array(&mut self, l: &mut LuaState, table_ptr: TablePtr) -> bool {
        let gc_table = table_ptr.as_ref();
        let table = &gc_table.data;
        let array_len = table.len();
        let mut marked = false;
        // Mark all array entries
        for i in 1..=array_len {
            if let Some(value) = table.raw_geti(i as i64) {
                if let Some(gc_ptr) = value.as_gc_ptr() {
                    if self.is_white(gc_ptr) {
                        marked = true;
                        self.mark_object(l, gc_ptr);
                    }
                }
            }
        }

        marked
    }

    /// Traverse a strong (non-weak) table - mark everything
    fn traverse_strong_table(&mut self, l: &mut LuaState, table_ptr: TablePtr) {
        let gc_table = table_ptr.as_mut_ref();
        let table = &gc_table.data;

        // Use for_each_entry() to iterate all entries (both array and hash parts)
        // This avoids both allocating Vec (iter_all) and repeated lookups (next)
        // Port of Lua 5.5's direct pointer iteration: `for (n = gnode(h, 0); n < limit; n++)`
        table.for_each_entry(|k, v| {
            if let Some(k_ptr) = k.as_gc_ptr() {
                self.mark_object(l, k_ptr);
            }
            if let Some(v_ptr) = v.as_gc_ptr() {
                self.mark_object(l, v_ptr);
            }
        });

        self.gen_link(table_ptr.into());
    }

    // static void genlink (global_State *g, GCObject *o) {
    //     lua_assert(isblack(o));
    //     if (getage(o) == G_TOUCHED1) {  /* touched in this cycle? */
    //         linkobjgclist(o, g->grayagain);  /* link it back in 'grayagain' */
    //     }  /* everything else do not need to be linked back */
    //     else if (getage(o) == G_TOUCHED2)
    //         setage(o, G_OLD);  /* advance age */
    //     }
    // }
    fn gen_link(&mut self, gc_ptr: GcObjectPtr) {
        let Some(header) = gc_ptr.header_mut() else {
            return;
        };
        debug_assert!(header.is_black(), "genlink called on non-black object");

        if header.age() == G_TOUCHED1 {
            // Touched in this cycle, link back to grayagain
            self.grayagain.push(gc_ptr);
        } else if header.age() == G_TOUCHED2 {
            // Advance age to G_OLD
            header.set_age(G_OLD);
        }
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
    fn traverse_weak_value(&mut self, l: &mut LuaState, table_ptr: TablePtr) {
        let gc_table = table_ptr.as_ref();
        let table = &gc_table.data;

        let mut has_clears = table.len() > 0;

        // CRITICAL FIX: Use next() instead of iter_all() to avoid allocation
        let mut key = LuaValue::nil();
        while let Some((k, v)) = table.next(&key).unwrap_or(None) {
            // Mark key (strong reference)
            if let Some(key_ptr) = k.as_gc_ptr() {
                self.mark_object(l, key_ptr);
            }

            if let Some(val_ptr) = v.as_gc_ptr() {
                // Re-check if value is STILL white after marking the key
                if !has_clears && self.is_cleared(l, val_ptr) {
                    has_clears = true;
                }
            }
            key = k;
        }

        if self.gc_state == GcState::Propagate {
            // During propagation phase, keep in grayagain for atomic phase
            self.grayagain.push(table_ptr.into());
        } else if has_clears {
            // In atomic phase, if has white values, add to weak list for clearing
            self.weak.push(table_ptr.into());
        } else {
            // Otherwise, genlink to check age
            self.gen_link(table_ptr.into());
        }
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
    fn traverse_ephemeron(&mut self, l: &mut LuaState, table_ptr: TablePtr) -> bool {
        let gc_table = table_ptr.as_mut_ref();
        let table = &gc_table.data;
        let mut has_clears = false;
        let mut has_ww = false; // white->white

        let mut marked = self.traverse_array(l, table_ptr);

        let mut key = LuaValue::nil();
        while let Some((k, v)) = table.next(&key).unwrap_or(None) {
            let key_ptr = k.as_gc_ptr();
            let val_ptr = v.as_gc_ptr();

            // Check if key is cleared (iscleared will mark strings)
            let key_is_cleared = key_ptr.map_or(false, |ptr| self.is_cleared(l, ptr));
            let val_is_white = val_ptr.map_or(false, |ptr| self.is_white(ptr));
            if key_is_cleared {
                has_clears = true;
                if val_is_white {
                    has_ww = true;
                }
            } else if val_is_white {
                marked = true;
                // Key is alive, but value is white - mark the value
                self.really_mark_object(l, val_ptr.unwrap());
            }
            key = k;
        }

        if self.gc_state == GcState::Propagate {
            // During propagation phase, keep in grayagain for atomic phase
            self.grayagain.push(table_ptr.into());
        } else if has_ww {
            // In atomic phase, if has white->white entries, add to ephemeron list
            self.ephemeron.push(table_ptr.into());
        } else if has_clears {
            // If has cleared keys, add to allweak list for clearing
            self.allweak.push(table_ptr.into());
        } else {
            // Otherwise, genlink to check age
            self.gen_link(table_ptr.into());
        }

        marked
    }

    fn traverse_table(&mut self, l: &mut LuaState, table_ptr: TablePtr) -> usize {
        // Port of Lua 5.5's traversetable with weak table handling
        // Check weak mode first to decide how to traverse
        let gc_table = table_ptr.as_mut_ref();
        gc_table.header.make_black();

        if let Some(metatable) = gc_table.data.get_metatable() {
            self.mark_object(l, metatable.as_gc_ptr().unwrap());
        }

        let weak_mode = self.get_weak_mode(table_ptr);

        match weak_mode {
            None | Some((false, false)) => {
                // Regular table (or invalid weak mode) - mark everything
                self.traverse_strong_table(l, table_ptr);
            }
            Some((false, true)) => {
                // Weak values only (__mode = 'v')
                self.traverse_weak_value(l, table_ptr);
            }
            Some((true, false)) => {
                // Weak keys only (__mode = 'k') - ephemeron
                self.traverse_ephemeron(l, table_ptr);
            }
            Some((true, true)) => {
                // Both weak (__mode = 'kv') - fully weak
                if self.gc_state == GcState::Propagate {
                    // During propagation phase, keep in grayagain for atomic phase
                    self.grayagain.push(table_ptr.into());
                } else {
                    // In atomic phase, add to allweak list for clearing
                    self.allweak.push(table_ptr.into());
                }
            }
        }

        // Estimate work done: 1 + total entries (array + hash)
        1 + gc_table.data.len() + gc_table.data.hash_size()
    }

    fn traverse_function(&mut self, l: &mut LuaState, func_ptr: FunctionPtr) -> usize {
        // Mark the function black and get references to data we need
        // (Fixed functions should never reach here - they stay gray forever
        let gc_func = func_ptr.as_mut_ref();
        gc_func.header.make_black();

        let mut count = 1; // Estimate of work done
        let func_body = &gc_func.data;
        let upvalues = func_body.upvalues();
        count += upvalues.len();
        // Mark upvalues
        for upval_ptr in upvalues {
            self.mark_object(l, upval_ptr.clone().into());
        }

        // Mark all constants in the chunk and nested chunks (like Lua 5.5's traverseproto)
        count += self.mark_chunk_constants(l, gc_func.data.chunk());

        count as usize // Estimate of work done
    }

    fn traverse_cclosure(&mut self, l: &mut LuaState, closure_ptr: CClosurePtr) -> usize {
        // Mark the C closure black and get references to data we need
        let gc_closure = closure_ptr.as_mut_ref();
        gc_closure.header.make_black();

        let mut count = 1; // Estimate of work done
        let upvalues = gc_closure.data.upvalues();
        count += upvalues.len();
        // Mark upvalues
        for upval in upvalues {
            self.mark_value(l, upval);
        }

        count as usize // Estimate of work done
    }

    fn traverse_thread(&mut self, l: &mut LuaState, thread_ptr: ThreadPtr) -> usize {
        // Mark the thread black and get references to data we need
        let gc_thread = thread_ptr.as_mut_ref();
        gc_thread.header.make_black();

        let mut count = 1; // Estimate of work done
        if gc_thread.header.is_old() || self.gc_state == GcState::Propagate {
            self.grayagain.push(thread_ptr.into());
        }

        {
            let state = &gc_thread.data;

            let top = state.get_top();
            let stack = state.stack();

            for i in 0..top {
                self.mark_value(l, &stack[i]);
                count += 1;
            }

            for open_upval_ptr in state.open_upvalues() {
                self.mark_object(l, open_upval_ptr.clone().into());
            }
        } // Drop immutable borrow of gc_thread.data

        if self.gc_state == GcState::Atomic {
            if !self.gc_emergency {
                // luaD_shrinkstack(th); /* do not change stack in emergency cycle */
                // TODO: implement stack shrinking if needed
            }

            // Lua 5.5 lgc.c traversethread atomic phase:
            // for (o = th->top.p; o < th->stack_last.p + EXTRA_STACK; o++)
            //     setnilvalue(s2v(o));  /* clear dead stack slice */
            // This clears stale/dead values above the current stack top so that
            // future set_top growth won't expose dangling references.
            {
                let state = &mut gc_thread.data;
                let stack_top = state.get_top();
                let stack_len = state.stack_len();
                let stack = state.stack_mut();
                for i in stack_top..stack_len {
                    stack[i] = LuaValue::nil();
                }
            }

            if !self.is_in_twups(thread_ptr) && !gc_thread.data.open_upvalues().is_empty() {
                self.twups.push(thread_ptr);
            }
        } else {
            let state = &gc_thread.data;
            if !self.is_in_twups(thread_ptr) && !state.open_upvalues().is_empty() {
                self.twups.push(thread_ptr);
            }
        }

        count as usize // Estimate of work done
    }

    fn is_in_twups(&self, thread_ptr: ThreadPtr) -> bool {
        // Check if the thread is in the twups list
        self.twups.contains(&thread_ptr)
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
    /// /*
    // ** traverse one gray object, turning it to black. Return an estimate
    // ** of the number of slots traversed.
    // */
    // static l_mem propagatemark (global_State *g) {
    //   GCObject *o = g->gray;
    //   nw2black(o);
    //   g->gray = *getgclist(o);  /* remove from 'gray' list */
    //   switch (o->tt) {
    //     case LUA_VTABLE: return traversetable(g, gco2t(o));
    //     case LUA_VUSERDATA: return traverseudata(g, gco2u(o));
    //     case LUA_VLCL: return traverseLclosure(g, gco2lcl(o));
    //     case LUA_VCCL: return traverseCclosure(g, gco2ccl(o));
    //     case LUA_VPROTO: return traverseproto(g, gco2p(o));
    //     case LUA_VTHREAD: return traversethread(g, gco2th(o));
    //     default: lua_assert(0); return 0;
    //   }
    // }
    fn propagate_mark(&mut self, l: &mut LuaState) -> isize {
        if let Some(gc_ptr) = self.gray.pop() {
            self.propagate_mark_one(l, gc_ptr) as isize
        } else {
            0
        }
    }

    /// Mark one object and traverse its references
    /// Like Lua 5.5's propagatemark: "nw2black(o);" then traverse
    /// Sets object to BLACK before traversing children
    fn propagate_mark_one(&mut self, l: &mut LuaState, gc_ptr: GcObjectPtr) -> usize {
        match gc_ptr {
            GcObjectPtr::Table(table_ptr) => self.traverse_table(l, table_ptr),
            GcObjectPtr::Function(func_ptr) => self.traverse_function(l, func_ptr),
            GcObjectPtr::CClosure(closure_ptr) => self.traverse_cclosure(l, closure_ptr),
            GcObjectPtr::Userdata(userdata_ptr) => {
                // Userdata: mark the userdata itself and its metatable if any
                let gc_ud = userdata_ptr.as_mut_ref();
                gc_ud.header.make_black();
                if let Some(metatable) = gc_ud.data.get_metatable() {
                    // Mark metatable if exists (it's a LuaValue, could be table)
                    self.mark_value(l, &metatable);
                }

                self.gen_link(gc_ptr);

                1 // Estimate of work done
            }
            GcObjectPtr::Thread(thread_ptr) => self.traverse_thread(l, thread_ptr),
            _ => 0,
        }
    }

    /// Atomic phase (like atomic in Lua 5.5)
    /// Atomic phase of GC - mark-and-sweep in one uninterruptible step
    /// Port of Lua 5.5's atomic function from lgc.c
    fn atomic(&mut self, l: &mut LuaState) {
        self.gc_state = GcState::Atomic;

        // Mark main thread (running thread)
        let main_thread_ptr = l.vm_mut().get_main_thread_ptr();
        self.mark_object(l, main_thread_ptr.into());

        // Mark registry (global state)
        let registry = l.vm_mut().registry;
        self.mark_value(l, &registry);

        // Mark global metatables (string, number, etc.)
        self.mark_mt(l);

        // Propagate all marks (empty the gray list)
        self.propagate_all(l);

        self.remark_upvalues(l);

        // Propagate changes from remark_upvalues
        self.propagate_all(l);

        let grayagain = std::mem::take(&mut self.grayagain);
        self.gray = grayagain;
        self.propagate_all(l);

        self.converge_ephemerons(l);

        /* at this point, all strongly accessible objects are marked. */
        /* Clear values from weak tables, before checking finalizers */
        self.clear_by_values(l);

        /* separate objects to be finalized */
        self.separate_to_be_finalized(false);

        /* mark objects that will be finalized */
        self.mark_being_finalized(l);

        /* remark, to propagate 'resurrection' */
        self.propagate_all(l);

        self.converge_ephemerons(l);

        self.clear_by_keys(l);

        // Clear values again (for resurrected weak tables)
        // Lua 5.5 iterates 'weak' and 'allweak' lists again here
        self.clear_by_values(l);

        self.current_white = GcHeader::otherwhite(self.current_white); // Flip current white

        debug_assert!(
            self.gray.is_empty(),
            "Gray list should be empty at end of atomic phase"
        );
    }

    fn propagate_all(&mut self, l: &mut LuaState) {
        while !self.gray.is_empty() {
            self.propagate_mark(l);
        }
    }

    /// Port of Lua 5.5's remarkupvals from lgc.c
    ///
    /// ```c
    /// static void remarkupvals (global_State *g) {
    ///   lua_State *thread;
    ///   lua_State **p = &g->twups;
    ///   while ((thread = *p) != NULL) {
    ///     lua_assert(!iswhite(thread));  /* threads are never white */
    ///     if (isgray(thread) && thread->openupval != NULL)
    ///       p = &thread->twups;  /* keep marked thread with upvalues in the list */
    ///     else {  /* thread is black or has no upvalues */
    ///       UpVal *uv;
    ///       *p = thread->twups;  /* remove thread from the list */
    ///       thread->twups = thread;  /* mark that it is out of list */
    ///       for (uv = thread->openupval; uv != NULL; uv = uv->u.open.next) {
    ///         lua_assert(getage(uv) <= getage(thread));
    ///         if (!iswhite(uv))
    ///           markvalue(g, uv->v.p);  /* remark upvalue's value */
    ///       }
    ///     }
    ///   }
    /// }
    /// ```
    fn remark_upvalues(&mut self, l: &mut LuaState) {
        let mut i = 0;
        while i < self.twups.len() {
            let thread_ptr = self.twups[i];
            let thread = thread_ptr.as_ref();

            // White thread = dead (unreachable). Remove from list.
            // Gray thread with open upvalues = keep in list for later remarking.
            // Otherwise (black or no upvalues) = remove and remark its upvalues.
            if thread.header.is_white() {
                // Thread is dead, just remove from list
                self.twups.swap_remove(i);
                continue;
            }

            // if so, just move to the next thread
            if thread.header.is_gray() && !thread.data.open_upvalues().is_empty() {
                i += 1;
                continue;
            }

            // else, thread is black or has no upvalues
            // note: swap_remove moves last element to index i
            self.twups.swap_remove(i);

            // remark upvalues
            for upval_ptr in thread.data.open_upvalues() {
                let upval = upval_ptr.as_ref();

                // Upvalue age should not be older than its thread
                debug_assert!(
                    upval.header.age() <= thread.header.age(),
                    "Upvalue should not be older than its thread"
                );

                if !upval.header.is_white() {
                    // get value from upvalue and mark it
                    let value = upval.data.get_value();
                    self.mark_value(l, &value);
                }
            }
        }
    }

    /// Enter sweep phase (like entersweep in Lua 5.5)
    pub fn enter_sweep(&mut self, _l: &mut LuaState) {
        self.gc_state = GcState::SwpAllGc;
        // sweeptoalive
        self.sweepgc = SweepGc::AllGc(0);
    }

    /// Sweep step - collect dead objects (like sweepstep in Lua 5.5)
    fn sweep_step(
        &mut self,
        l: &mut LuaState,
        next_state: GcState,
        next_sweepgc: SweepGc,
        fast: bool,
    ) {
        if !self.sweepgc.is_done() {
            self.sweep_list(
                l,
                if fast {
                    std::usize::MAX
                } else {
                    GCSWEEPMAX as usize
                },
            );
        } else {
            self.gc_state = next_state;
            self.sweepgc = next_sweepgc;
        }
    }

    /// Sweep a list of objects, freeing dead ones and resetting survivors
    /// Port of Lua 5.5's sweeplist from lgc.c
    ///
    /// Lua 5.5 源码：
    /// ```c
    /// static GCObject **sweeplist (lua_State *L, GCObject **p, l_mem countin) {
    ///   global_State *g = G(L);
    ///   int ow = otherwhite(g);
    ///   int white = luaC_white(g);  /* current white */
    ///   while (*p != NULL && countin-- > 0) {
    ///     GCObject *curr = *p;
    ///     int marked = curr->marked;
    ///     if (isdeadm(ow, marked)) {  /* is 'curr' dead? */
    ///       *p = curr->next;  /* remove 'curr' from list */
    ///       freeobj(L, curr);  /* erase 'curr' */
    ///     }
    ///     else {  /* change mark to 'white' and age to 'new' */
    ///       curr->marked = cast_byte((marked & ~maskgcbits) | white | G_NEW);
    ///       p = &curr->next;  /* go to next element */
    ///     }
    ///   }
    ///   return (*p == NULL) ? NULL : p;
    /// }
    /// ```
    ///
    fn sweep_list(&mut self, l: &mut LuaState, mut sweep_count: usize) {
        let other_white = 1 - self.current_white;

        // In incremental mode, we sweep all three generation lists sequentially
        // 根据 sweepgc 状态决定操作哪个列表
        match &mut self.sweepgc {
            SweepGc::AllGc(index) => {
                // Phase 1: Sweep allgc (G_NEW objects)
                while *index < self.allgc.len() && sweep_count > 0 {
                    let gc_ptr = self.allgc.get(*index).unwrap().as_gc_ptr();

                    if let Some(header) = gc_ptr.header() {
                        // 检查是否是死对象（other white）
                        if header.is_dead(other_white) {
                            // BUG FIX: Check FINALIZEDBIT - objects with finalizers must
                            // go to tobefnz, not be freed, to avoid dangling finobj pointers
                            if header.to_finalize() {
                                self.tobefnz.push(gc_ptr);
                                if let Some(header_mut) = gc_ptr.header_mut() {
                                    header_mut.make_white(self.current_white);
                                    header_mut.set_age(G_NEW);
                                }
                                *index += 1;
                            } else {
                                if let GcObjectPtr::String(str_ptr) = gc_ptr {
                                    Self::remove_dead_string_from_intern(l, str_ptr);
                                }

                                let obj = self.allgc.remove(gc_ptr);
                                self.total_bytes -= obj.size() as isize;
                                GC::release_object_static(obj);
                            }
                        } else {
                            // 存活对象：重置为当前白色 + G_NEW
                            if let Some(header_mut) = gc_ptr.header_mut() {
                                header_mut.make_white(self.current_white);
                                header_mut.set_age(G_NEW);
                            }
                            *index += 1;
                        }
                    } else {
                        *index += 1;
                    }

                    sweep_count -= 1;
                }

                // If allgc sweep complete, move to survival list
                if *index >= self.allgc.len() {
                    self.sweepgc = SweepGc::Survival(0);
                }
            }

            SweepGc::Survival(index) => {
                // Phase 2: Sweep survival list (G_SURVIVAL objects)
                while *index < self.survival.len() && sweep_count > 0 {
                    let gc_ptr = self.survival.get(*index).unwrap().as_gc_ptr();

                    if let Some(header) = gc_ptr.header() {
                        if header.is_dead(other_white) {
                            // BUG FIX: Check FINALIZEDBIT (same as AllGc phase)
                            if header.to_finalize() {
                                self.tobefnz.push(gc_ptr);
                                if let Some(header_mut) = gc_ptr.header_mut() {
                                    header_mut.make_white(self.current_white);
                                    header_mut.set_age(G_NEW);
                                }
                                *index += 1;
                            } else {
                                if let GcObjectPtr::String(str_ptr) = gc_ptr {
                                    Self::remove_dead_string_from_intern(l, str_ptr);
                                }

                                let obj = self.survival.remove(gc_ptr);
                                self.total_bytes -= obj.size() as isize;
                                GC::release_object_static(obj);
                            }
                        } else {
                            // 存活对象：重置为当前白色 + G_NEW，移回 allgc
                            if let Some(header_mut) = gc_ptr.header_mut() {
                                header_mut.make_white(self.current_white);
                                header_mut.set_age(G_NEW);
                            }
                            *index += 1;
                        }
                    } else {
                        *index += 1;
                    }

                    sweep_count -= 1;
                }

                // If survival sweep complete, move to old list
                if *index >= self.survival.len() {
                    self.sweepgc = SweepGc::Old(0);
                }
            }

            SweepGc::Old(index) => {
                // Phase 3: Sweep old list (G_OLD1, G_OLD, G_TOUCHED* objects)
                while *index < self.old.len() && sweep_count > 0 {
                    let gc_ptr = self.old.get(*index).unwrap().as_gc_ptr();

                    if let Some(header) = gc_ptr.header() {
                        if header.is_dead(other_white) {
                            // BUG FIX: Check FINALIZEDBIT (same as AllGc phase)
                            if header.to_finalize() {
                                self.tobefnz.push(gc_ptr);
                                if let Some(header_mut) = gc_ptr.header_mut() {
                                    header_mut.make_white(self.current_white);
                                    header_mut.set_age(G_NEW);
                                }
                                *index += 1;
                            } else {
                                if let GcObjectPtr::String(str_ptr) = gc_ptr {
                                    Self::remove_dead_string_from_intern(l, str_ptr);
                                }

                                let obj = self.old.remove(gc_ptr);
                                self.total_bytes -= obj.size() as isize;
                                GC::release_object_static(obj);
                            }
                        } else {
                            // 存活对象：重置为当前白色 + G_NEW，移回 allgc
                            if let Some(header_mut) = gc_ptr.header_mut() {
                                header_mut.make_white(self.current_white);
                                header_mut.set_age(G_NEW);
                            }
                            *index += 1;
                        }
                    } else {
                        *index += 1;
                    }

                    sweep_count -= 1;
                }

                // If old sweep complete, move to finobj
                if *index >= self.old.len() {
                    // In incremental mode, after sweeping all generation lists,
                    // we need to move surviving objects to allgc
                    // For now, just mark as Done (objects remain in their lists with G_NEW age)
                    self.sweepgc = SweepGc::Done;
                }
            }

            SweepGc::FinObj(index) => {
                // 扫描 finobj 列表（有终结器的对象）
                while *index < self.finobj.len() && sweep_count > 0 {
                    let gc_ptr = self.finobj[*index];

                    if let Some(header) = gc_ptr.header() {
                        if header.is_dead(other_white) {
                            // 死对象且有终结器：移到 tobefnz
                            self.tobefnz.push(gc_ptr);
                            self.finobj.swap_remove(*index);
                            // 不增加 index（因为 swap_remove）
                        } else {
                            // 存活对象：重置为当前白色 + G_NEW
                            if let Some(header_mut) = gc_ptr.header_mut() {
                                header_mut.make_white(self.current_white);
                                header_mut.set_age(G_NEW);
                            }
                            *index += 1;
                        }
                    } else {
                        *index += 1;
                    }

                    sweep_count -= 1;
                }

                if *index >= self.finobj.len() {
                    self.sweepgc = SweepGc::Done;
                }
            }

            SweepGc::ToBeFnz(index) => {
                // 扫描 tobefnz 列表（等待终结的对象）
                // 注意：tobefnz 中的对象不应该被清理，它们等待终结器调用
                while *index < self.tobefnz.len() && sweep_count > 0 {
                    let gc_ptr = self.tobefnz[*index];

                    // tobefnz 中的对象保持原状，只是重置颜色
                    if let Some(header_mut) = gc_ptr.header_mut() {
                        header_mut.make_white(self.current_white);
                        header_mut.set_age(G_NEW);
                    }

                    *index += 1;
                    sweep_count -= 1;
                }

                if *index >= self.tobefnz.len() {
                    self.sweepgc = SweepGc::Done;
                }
            }

            SweepGc::Done => {
                // 已经完成，不做任何事
            }
        }
    }

    fn sweep2old(&mut self, _l: &mut LuaState, to_clear_list: &mut HashSet<GcObjectPtr>) {
        //  Use other_white to distinguish dead objects from new objects
        // - current_white: new objects created in this cycle
        // - other_white: dead objects from previous cycle
        // Port of Lua 5.5 lgc.c sweep2old: uses isdeadm(ow, marked)
        let other_white = GcHeader::otherwhite(self.current_white);

        // Helper closure to process a single object
        let process_object = |gc_ptr: GcObjectPtr,
                              to_clear: &mut HashSet<GcObjectPtr>,
                              grayagain: &mut Vec<GcObjectPtr>|
         -> bool {
            let Some(header) = gc_ptr.header() else {
                return false; // Keep object (no header)
            };

            // CRITICAL FIX: Use is_dead(other_white) instead of is_white()
            if header.is_dead(other_white) {
                to_clear.insert(gc_ptr);
                true // Mark for removal
            } else {
                if let Some(header_mut) = gc_ptr.header_mut() {
                    header_mut.set_age(G_OLD);
                    if gc_ptr.kind() == GcObjectKind::Thread {
                        grayagain.push(gc_ptr);
                    } else if let GcObjectPtr::Upvalue(upval_ptr) = gc_ptr {
                        let gc_upval = upval_ptr.as_mut_ref();
                        if gc_upval.data.is_open() {
                            header_mut.make_gray();
                        } else {
                            header_mut.make_black();
                        }
                    } else {
                        header_mut.make_black();
                    }
                }
                false // Keep object
            }
        };

        // Process allgc list (G_NEW objects)
        let mut i = self.allgc.len();
        while i > 0 {
            i -= 1;
            if let Some(obj) = self.allgc.get(i) {
                let gc_ptr = obj.as_gc_ptr();
                process_object(gc_ptr, to_clear_list, &mut self.grayagain);
            }
        }

        // Process survival list (G_SURVIVAL objects)
        let mut i = self.survival.len();
        while i > 0 {
            i -= 1;
            if let Some(obj) = self.survival.get(i) {
                let gc_ptr = obj.as_gc_ptr();
                process_object(gc_ptr, to_clear_list, &mut self.grayagain);
            }
        }

        // Process old list (G_OLD, G_TOUCHED* objects)
        let mut i = self.old.len();
        while i > 0 {
            i -= 1;
            if let Some(obj) = self.old.get(i) {
                let gc_ptr = obj.as_gc_ptr();
                process_object(gc_ptr, to_clear_list, &mut self.grayagain);
            }
        }

        // Process old1 list (G_OLD1 objects)
        let mut i = self.old1.len();
        while i > 0 {
            i -= 1;
            if let Some(obj) = self.old1.get(i) {
                let gc_ptr = obj.as_gc_ptr();
                process_object(gc_ptr, to_clear_list, &mut self.grayagain);
            }
        }

        // After sweep2old, all surviving objects are G_OLD and should be in old list
        // Move all surviving objects from allgc, survival and old1 to old
        while !self.allgc.is_empty() {
            // Get the last object (we can't iterate while modifying)
            let gc_ptr = self.allgc.get(0).unwrap().as_gc_ptr();
            if !to_clear_list.contains(&gc_ptr) {
                let owner = self.allgc.remove(gc_ptr);
                self.old.add(owner);
            } else {
                // Dead object will be removed later, just break out
                break;
            }
        }

        while !self.survival.is_empty() {
            let gc_ptr = self.survival.get(0).unwrap().as_gc_ptr();
            if !to_clear_list.contains(&gc_ptr) {
                let owner = self.survival.remove(gc_ptr);
                self.old.add(owner);
            } else {
                break;
            }
        }

        while !self.old1.is_empty() {
            let gc_ptr = self.old1.get(0).unwrap().as_gc_ptr();
            if !to_clear_list.contains(&gc_ptr) {
                let owner = self.old1.remove(gc_ptr);
                self.old.add(owner);
            } else {
                break;
            }
        }
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
    pub fn run_until_state(&mut self, l: &mut LuaState, target_state: GcState) {
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
            self.single_step(l, true);
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
    pub fn full_generation(&mut self, l: &mut LuaState) {
        // If we're in Pause state, we need to do the state transition ourselves
        if self.gc_state == GcState::Pause {
            //  Call restart_collection WHILE STILL IN PAUSE STATE!
            // Lua 5.5's singlestep calls restartcollection() while gcstate==GCSpause,
            // THEN sets gcstate=GCSpropagate.
            self.restart_collection(l);

            // NOW transition to Propagate state (like Lua 5.5's singlestep)
            self.gc_state = GcState::Propagate;
        }

        // Step 2: Propagate gray list (weak tables will be added to grayagain)
        while !self.gray.is_empty() {
            self.propagate_mark(l);
        }

        // Step 3: Call atomic phase (like youngcollection does)
        // atomic() will process grayagain, converge ephemerons, and clear weak tables
        self.atomic(l);

        // Step 4: Sweep all generation lists (full GC sweeps EVERYTHING)
        self.gc_state = GcState::SwpAllGc;
        self.sweep_full_gen(l);

        // Step 5: Call finalizers
        self.gc_state = GcState::CallFin;

        // Call all pending finalizers (like full_inc does)
        while !self.tobefnz.is_empty() && !self.gc_emergency {
            self.call_one_finalizer(l);
        }

        // Return to Pause
        self.gc_state = GcState::Pause;

        // set_pause uses total_bytes (actual memory after sweep) as base
        self.set_pause();
    }

    /// Sweep all objects in full generational GC
    /// Unlike sweep_gen (young collection), this sweeps ALL generations
    /// Dead objects are freed, survivors become G_OLD (no promotion chain)
    fn sweep_full_gen(&mut self, l: &mut LuaState) {
        let other_white = 1 - self.current_white;
        let current_white = self.current_white;

        // Helper: sweep a list, freeing dead objects (respecting FINALIZEDBIT)
        // and keeping survivors as G_OLD
        let sweep_full_list = |list: &mut GcList,
                               total_bytes: &mut isize,
                               tobefnz: &mut Vec<GcObjectPtr>,
                               l: &mut LuaState|
         -> Vec<GcObjectOwner> {
            let objects = list.take_all();
            let mut survivors: Vec<GcObjectOwner> = Vec::new();

            for mut gc_owner in objects {
                let gc_ptr = gc_owner.as_gc_ptr();
                let Some(header) = gc_ptr.header() else {
                    continue;
                };

                if header.is_dead(other_white) {
                    // BUG FIX: Check FINALIZEDBIT before freeing
                    if header.to_finalize() {
                        gc_owner.header_mut().set_age(G_OLD);
                        gc_owner.header_mut().make_white(current_white);
                        tobefnz.push(gc_ptr);
                        survivors.push(gc_owner);
                    } else {
                        if let GcObjectPtr::String(str_ptr) = gc_ptr {
                            GC::remove_dead_string_from_intern(l, str_ptr);
                        }
                        *total_bytes -= gc_owner.size() as isize;
                        GC::release_object_static(gc_owner);
                    }
                } else {
                    gc_owner.header_mut().set_age(G_OLD);
                    gc_owner.header_mut().make_white(current_white);
                    survivors.push(gc_owner);
                }
            }

            survivors
        };

        let allgc_survivors =
            sweep_full_list(&mut self.allgc, &mut self.total_bytes, &mut self.tobefnz, l);
        let survival_survivors = sweep_full_list(
            &mut self.survival,
            &mut self.total_bytes,
            &mut self.tobefnz,
            l,
        );
        let old_survivors =
            sweep_full_list(&mut self.old, &mut self.total_bytes, &mut self.tobefnz, l);
        let old1_survivors =
            sweep_full_list(&mut self.old1, &mut self.total_bytes, &mut self.tobefnz, l);

        // All survivors go to old list
        self.old.add_all(allgc_survivors);
        self.old.add_all(survival_survivors);
        self.old.add_all(old_survivors);
        self.old.add_all(old1_survivors);
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
            self.gc_marked.max(512 * 1024) // 512KB minimum for first few collections
        };
        let debt = self.apply_param(MINORMUL, base);
        self.set_debt(debt);
    }

    /// Age transition function (replaces Lua 5.5's nextage array)
    /// Inlined for performance - compiler will optimize to jump table or branches
    #[inline]
    fn next_age(age: u8) -> u8 {
        match age {
            G_NEW => G_SURVIVAL,
            G_SURVIVAL => G_OLD1,
            G_OLD0 => G_OLD1,
            G_OLD1 => G_OLD,
            G_OLD => G_OLD,
            G_TOUCHED1 => G_TOUCHED1,
            G_TOUCHED2 => G_TOUCHED2,
            _ => age,
        }
    }

    fn young_collection(&mut self, l: &mut LuaState) {
        self.stats.minor_collections += 1;

        //  Set gc_stopem to prevent recursive GC during collection
        // This matches Lua 5.5's behavior where GC steps check gc_stopem
        let old_stopem = self.gc_stopem;
        self.gc_stopem = true;

        let marked = self.gc_marked; // Preserve gc_marked

        // Ensure we're in the right state for young collection
        // If not in Propagate, start from restart
        if self.gc_state != GcState::Propagate {
            self.restart_collection(l);
            self.gc_state = GcState::Propagate;
            // No need to rebuild young_objects - we use generation lists now
        }

        // Phase 1: Mark OLD1 objects (including finobj and tobefnz)
        self.mark_old(l);

        // Phase 2: Atomic phase
        self.atomic(l);

        // Phase 3: Sweep young generation and track promoted bytes
        self.gc_state = GcState::SwpAllGc;
        let added_old1 = self.sweep_gen(l);

        // Phase 4: Update gc_marked with promoted bytes
        self.gc_marked = marked + added_old1;

        // Phase 5: Check if need to switch to major mode
        // Skip this check on first generation (gc_majorminor == 0 means first time)
        let is_first_gen = self.gc_majorminor == 0;
        if is_first_gen {
            self.gc_majorminor = self.gc_marked; // Initialize for next cycle
            self.finish_gen_cycle(l);
        } else if self.check_major_minor(l) {
            // self.minor_to_incremental(l, KGC_GENMAJOR);
            self.gc_marked = 0; // Avoid pause in first major cycle
        } else {
            self.finish_gen_cycle(l); // Still in minor mode; finish it
        }

        // Restore gc_stopem
        self.gc_stopem = old_stopem;
    }

    /// Sweep the young generation, promoting survivors
    /// Port of Lua 5.5's sweepgen function
    ///
    /// NEW DESIGN: Use separate GcLists per generation for O(young) sweep
    /// - allgc (G_NEW): Dead removed, survivors move to survival
    /// - survival (G_SURVIVAL): Dead removed, survivors move to old
    /// - old: Only G_OLD1 are processed by mark_old, others are skipped
    ///
    /// Returns: bytes promoted to OLD1 generation
    fn sweep_gen(&mut self, l: &mut LuaState) -> isize {
        let other_white = 1 - self.current_white;
        let mut added_old1: isize = 0;

        // Phase 1: Sweep allgc list (G_NEW objects)
        // Dead objects are freed, survivors are promoted to survival (G_SURVIVAL)
        let allgc_objects = self.allgc.take_all();
        let mut new_survival: Vec<GcObjectOwner> = Vec::new();

        for mut gc_owner in allgc_objects {
            let gc_ptr = gc_owner.as_gc_ptr();

            let Some(header) = gc_ptr.header() else {
                continue;
            };

            if header.is_dead(other_white) {
                // BUG FIX: Check FINALIZEDBIT before freeing.
                // Objects with finalizers are tracked in both generation lists AND finobj.
                // If we free them here, finobj will have dangling pointers → use-after-free.
                // Instead, move them to tobefnz for later finalization (like Lua 5.5).
                if header.to_finalize() {
                    // Has finalizer: keep alive for finalization
                    gc_owner.header_mut().make_white(self.current_white);
                    gc_owner.header_mut().set_age(G_SURVIVAL);
                    self.tobefnz.push(gc_ptr);
                    new_survival.push(gc_owner);
                } else {
                    // No finalizer: safe to free
                    if let GcObjectPtr::String(str_ptr) = gc_ptr {
                        Self::remove_dead_string_from_intern(l, str_ptr);
                    }
                    self.total_bytes -= gc_owner.size() as isize;
                    self.release_object(gc_owner);
                }
            } else {
                // Alive: promote to G_SURVIVAL and make white
                gc_owner.header_mut().set_age(G_SURVIVAL);
                gc_owner.header_mut().make_white(self.current_white);
                new_survival.push(gc_owner);
            }
        }

        // Phase 2: Sweep survival list (G_SURVIVAL objects)
        // Dead objects are freed, survivors are promoted to old1 (G_OLD1)
        let survival_objects = self.survival.take_all();
        let mut new_old1: Vec<GcObjectOwner> = Vec::new();

        for mut gc_owner in survival_objects {
            let gc_ptr = gc_owner.as_gc_ptr();

            let Some(header) = gc_ptr.header() else {
                continue;
            };

            if header.is_dead(other_white) {
                // BUG FIX: Check FINALIZEDBIT before freeing (same as Phase 1)
                if header.to_finalize() {
                    gc_owner.header_mut().make_white(self.current_white);
                    gc_owner.header_mut().set_age(G_OLD1);
                    self.tobefnz.push(gc_ptr);
                    let size = gc_owner.header().size as isize;
                    added_old1 += size;
                    new_old1.push(gc_owner);
                } else {
                    // No finalizer: safe to free
                    if let GcObjectPtr::String(str_ptr) = gc_ptr {
                        Self::remove_dead_string_from_intern(l, str_ptr);
                    }
                    self.total_bytes -= gc_owner.size() as isize;
                    self.release_object(gc_owner);
                }
            } else {
                // Alive: promote to G_OLD1 and KEEP COLOR (don't make white)
                let size = header.size as isize;
                gc_owner.header_mut().set_age(G_OLD1);
                // Track bytes becoming OLD1
                added_old1 += size;
                new_old1.push(gc_owner);
            }
        }

        // Restore the lists with surviving objects
        // New G_NEW objects will be added to allgc during this cycle
        // allgc is now empty - ready for new allocations
        self.survival.add_all(new_survival);
        self.old1.add_all(new_old1); // Now go to old1, not old

        // Process finobj and tobefnz lists
        added_old1 += self.sweep_gen_finobj();

        added_old1
    }

    /// Sweep finobj list in generational mode
    /// Returns: bytes promoted to OLD1 generation
    fn sweep_gen_finobj(&mut self) -> isize {
        let other_white = 1 - self.current_white;
        let mut added_old1: isize = 0;
        let mut i = 0;

        while i < self.finobj.len() {
            let gc_ptr = self.finobj[i];

            if let Some(header) = gc_ptr.header() {
                let age = header.age();

                if age == G_NEW {
                    if header.is_dead(other_white) {
                        // Dead with finalizer: move to tobefnz
                        self.tobefnz.push(gc_ptr);
                        self.finobj.swap_remove(i);
                        continue;
                    } else {
                        // Alive: promote to G_SURVIVAL and make white
                        if let Some(header_mut) = gc_ptr.header_mut() {
                            header_mut.set_age(Self::next_age(age));
                            header_mut.make_white(self.current_white);
                        }
                    }
                } else if age == G_SURVIVAL || age == G_OLD0 {
                    if header.is_dead(other_white) {
                        // Dead: move to tobefnz
                        self.tobefnz.push(gc_ptr);
                        self.finobj.swap_remove(i);
                        continue;
                    } else {
                        // Alive: promote to G_OLD1 and keep color
                        if let Some(header_mut) = gc_ptr.header_mut() {
                            let old_age = header_mut.age();
                            let new_age = Self::next_age(old_age);
                            header_mut.set_age(new_age);

                            // Track bytes becoming OLD1
                            if new_age == G_OLD1 {
                                if let Some(h) = gc_ptr.header() {
                                    added_old1 += h.size as isize;
                                }
                            }
                        }
                    }
                }
            }

            i += 1;
        }

        // Also sweep tobefnz list
        let mut j = 0;
        while j < self.tobefnz.len() {
            let gc_ptr = self.tobefnz[j];

            if let Some(header) = gc_ptr.header_mut() {
                let age = header.age();
                if age < G_OLD && !header.is_dead(other_white) {
                    // Resurrected or still alive: advance age
                    let new_age = Self::next_age(age);
                    header.set_age(new_age);

                    if new_age == G_OLD1 {
                        if let Some(h) = gc_ptr.header() {
                            added_old1 += h.size as isize;
                        }
                    }
                }
            }

            j += 1;
        }

        added_old1
    }

    // ============ GC Write Barriers (from lgc.c) ============

    /// Forward barrier (luaC_barrier_)
    /// Called when a black object 'o' is modified to point to white object 'v'
    /// This maintains the invariant: black objects cannot point to white objects
    pub fn barrier(&mut self, l: &mut LuaState, o_ptr: GcObjectPtr, v_ptr: GcObjectPtr) {
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
            self.mark_object(l, v_ptr);

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
    /// ```.
    fn really_mark_object(&mut self, l: &mut LuaState, gc_ptr: GcObjectPtr) {
        self.gc_marked += gc_ptr.header().map(|it| it.size as isize).unwrap_or(0);
        match gc_ptr {
            GcObjectPtr::String(_) | GcObjectPtr::Binary(_) => {
                gc_ptr.header_mut().unwrap().make_black(); // Leaves become black immediately
            }
            GcObjectPtr::Upvalue(upval_ptr) => {
                let uv = upval_ptr.as_mut_ref();
                if uv.data.is_open() {
                    uv.header.make_gray();
                } else {
                    uv.header.make_black();
                }

                let value = &uv.data.get_value();
                self.mark_value(l, value);
            }
            GcObjectPtr::Userdata(u_ptr) => {
                let ud = u_ptr.as_mut_ref();
                ud.header.make_black();
                if let Some(metatable) = ud.data.get_metatable() {
                    self.mark_object(l, metatable.as_gc_ptr().unwrap());
                }
            }
            _ => {
                let header = gc_ptr.header_mut().unwrap();
                //  Only add to gray list if not already gray
                // This prevents infinite loops in converge_ephemerons
                if !header.is_gray() {
                    header.make_gray(); // Others become gray
                    self.gray.push(gc_ptr);
                }
            }
        }
    }

    /// Mark an object (helper for barrier)
    fn mark_object(&mut self, l: &mut LuaState, gc_ptr: GcObjectPtr) {
        if let Some(header) = gc_ptr.header_mut() {
            // Only need to mark if it is white
            if header.is_white() {
                self.really_mark_object(l, gc_ptr);
            }
        }
    }

    /// Get the next object to be finalized from the 'tobefnz' list.
    /// Port of Lua 5.5's udata2finalize:
    /// ```c
    /// static GCObject *udata2finalize (global_State *g) {
    ///   GCObject *o = g->tobefnz;  /* get first element */
    ///   lua_assert(tofinalize(o));
    ///   g->tobefnz = o->next;  /* remove it from 'tobefnz' list */
    ///   o->next = g->allgc;  /* return it to 'allgc' list */
    ///   g->allgc = o;
    ///   resetbit(o->marked, FINALIZEDBIT);  /* object is "normal" again */
    ///   if (issweepphase(g))
    ///     makewhite(g, o);  /* "sweep" object */
    ///   else if (getage(o) == G_OLD1)
    ///     g->firstold1 = o;  /* it is the first OLD1 object in the list */
    ///   return o;
    /// }
    /// ```
    ///
    /// NOTE: In Rust with Vec pool, we don't physically move objects.
    /// The object stays in gc_list, we just remove it from tobefnz.
    fn udata2finalize(&mut self) -> Option<GcObjectPtr> {
        if self.tobefnz.is_empty() {
            return None;
        }

        // Get first element from tobefnz
        let gc_ptr = self.tobefnz.pop()?;

        // Reset FINALIZEDBIT (object is "normal" again)
        if let Some(header) = gc_ptr.header_mut() {
            header.clear_finalized();

            //  Make the object white with current_white color
            // This ensures that if the object is resurrected (referenced again)
            // during finalization, it won't be swept in the next GC cycle.
            // Lua 5.5 does this implicitly by calling resetbits which sets to
            // currentwhite.
            header.make_white(self.current_white);

            // If age is G_OLD1 in generational mode, note this
            // (In C, this updates g->firstold1 pointer)
            // In our Vec pool design, we don't need to track this explicitly
        }

        Some(gc_ptr)
    }

    /// Call ONE finalizer (__gc metamethod) for the next object in tobefnz.
    /// Port of Lua 5.5's GCTM:
    /// ```c
    /// static void GCTM (lua_State *L) {
    ///   global_State *g = G(L);
    ///   const TValue *tm;
    ///   TValue v;
    ///   lua_assert(!g->gcemergency);
    ///   setgcovalue(L, &v, udata2finalize(g));
    ///   tm = luaT_gettmbyobj(L, &v, TM_GC);
    ///   if (!notm(tm)) {  /* is there a finalizer? */
    ///     TStatus status;
    ///     lu_byte oldah = L->allowhook;
    ///     lu_byte oldgcstp  = g->gcstp;
    ///     g->gcstp |= GCSTPGC;  /* avoid GC steps */
    ///     L->allowhook = 0;  /* stop debug hooks during GC metamethod */
    ///     setobj2s(L, L->top.p++, tm);  /* push finalizer... */
    ///     setobj2s(L, L->top.p++, &v);  /* ... and its argument */
    ///     L->ci->callstatus |= CIST_FIN;  /* will run a finalizer */
    ///     status = luaD_pcall(L, dothecall, NULL, savestack(L, L->top.p - 2), 0);
    ///     L->ci->callstatus &= ~CIST_FIN;  /* not running a finalizer anymore */
    ///     L->allowhook = oldah;  /* restore hooks */
    ///     g->gcstp = oldgcstp;  /* restore state */
    ///     if (l_unlikely(status != LUA_OK)) {  /* error while running __gc? */
    ///       luaE_warnerror(L, "__gc");
    ///       L->top.p--;  /* pops error object */
    ///     }
    ///   }
    /// }
    /// ```
    fn call_one_finalizer(&mut self, l: &mut LuaState) {
        use crate::lua_vm::get_metamethod_event;
        debug_assert!(!self.gc_emergency, "GCTM called during emergency GC");

        // Get next object to finalize
        let Some(gc_ptr) = self.udata2finalize() else {
            return; // No more objects to finalize
        };

        // Convert GcObjectPtr to LuaValue
        let obj_value = match gc_ptr {
            GcObjectPtr::Table(ptr) => LuaValue::table(ptr),
            GcObjectPtr::Userdata(ptr) => LuaValue::userdata(ptr),
            GcObjectPtr::Thread(ptr) => LuaValue::thread(ptr),
            // Other types don't support __gc
            _ => {
                return;
            }
        };

        // Get __gc metamethod
        let Some(gc_method) = get_metamethod_event(l, &obj_value, TmKind::Gc) else {
            return; // No __gc metamethod
        };

        // Stop GC during finalization (g->gcstp |= GCSTPGC)
        // GCSTPGC prevents GC reentrancy by making collectgarbage() return false
        let old_stopem = self.gc_stopem;
        let old_debt = self.gc_debt;
        self.gc_stopem = true; // This is GCSTPGC, not GCSTPUSR (gc_stopped)

        // TODO: Save and restore L->allowhook (requires VM support)
        // TODO: Set L->ci->callstatus |= CIST_FIN (requires VM support)

        // Call __gc(obj) using pcall to handle errors safely
        let result = l.pcall(gc_method, vec![obj_value]);

        // Restore GC state
        self.gc_stopem = old_stopem;
        self.gc_debt = old_debt;

        // If error occurred, warn but don't propagate
        if let Err(_) = result {
            let msg = l.get_error_msg(LuaError::RuntimeError);
            eprintln!("[GC] WARNING: error in __gc: {}", msg);
        }
    }

    /// Call all pending finalizers (used in non-step contexts like finish_gen_cycle).
    /// This is NOT how Lua 5.5 normally runs finalizers (it uses GCTM one at a time),
    /// but useful for batch processing when appropriate.
    fn call_all_pending_finalizers(&mut self, l: &mut LuaState) {
        while !self.tobefnz.is_empty() && !self.gc_emergency {
            self.call_one_finalizer(l);
        }
    }

    // static void separatetobefnz (global_State *g, int all) {
    //     GCObject *curr;
    //     GCObject **p = &g->finobj;
    //     GCObject **lastnext = findlast(&g->tobefnz);
    //     while ((curr = *p) != g->finobjold1) {  /* traverse all finalizable objects */
    //         lua_assert(tofinalize(curr));
    //         if (!(iswhite(curr) || all))  /* not being collected? */
    //         p = &curr->next;  /* don't bother with it */
    //         else {
    //         if (curr == g->finobjsur)  /* removing 'finobjsur'? */
    //             g->finobjsur = curr->next;  /* correct it */
    //         *p = curr->next;  /* remove 'curr' from 'finobj' list */
    //         curr->next = *lastnext;  /* link at the end of 'tobefnz' list */
    //         *lastnext = curr;
    //         lastnext = &curr->next;
    //         }
    //     }
    // }
    fn separate_to_be_finalized(&mut self, all: bool) {
        let mut i = 0;
        while i < self.finobj.len() {
            let gc_ptr = self.finobj[i];

            let is_white = gc_ptr
                .header()
                .map(|header| header.is_white())
                .unwrap_or(false);

            if is_white || all {
                // Remove from finobj
                self.finobj.swap_remove(i);
                // Add to tobefnz
                self.tobefnz.push(gc_ptr);
            } else {
                i += 1; // Only increment if not removed
            }
        }
    }

    fn mark_being_finalized(&mut self, l: &mut LuaState) {
        for gc_ptr in self.tobefnz.clone() {
            self.mark_object(l, gc_ptr);
        }
    }

    pub fn get_error_message(&mut self) -> String {
        if let Some(msg) = std::mem::take(&mut self.gc_error_msg) {
            msg
        } else {
            String::new()
        }
    }

    pub fn disable_memory_check(&mut self) {
        self.gc_memory_check = false;
    }

    pub fn enable_memory_check(&mut self) {
        self.gc_memory_check = true;
    }

    pub fn check_memory(&mut self) -> LuaResult<()> {
        let total_bytes = self.get_total_bytes();
        let limit_bytes = self.get_limit_bytes();
        if total_bytes > limit_bytes {
            // For simple test, later will return an error instead of panic
            self.gc_error_msg = Some(format!(
                "Memory limit exceeded: {} bytes allocated (limit: {} bytes)",
                total_bytes, limit_bytes,
            ));
            return Err(LuaError::OutOfMemory);
        }

        Ok(())
    }
}

/// Result of a GC step
#[derive(Debug)]
enum StepResult {
    Work(isize), // Amount of work done
    Step2Pause,  // Reached pause state
    AtomicStep,  // Completed atomic phase
    Step2Minor,  // Returned to minor mode
}

impl Default for GC {
    fn default() -> Self {
        Self::new(SafeOption::default())
    }
}

#[derive(Debug)]
enum SweepGc {
    AllGc(usize),    // Sweeping allgc list (G_NEW objects)
    Survival(usize), // Sweeping survival list (G_SURVIVAL objects)
    Old(usize),      // Sweeping old list (G_OLD1, G_OLD, G_TOUCHED* objects)
    FinObj(usize),   // Sweeping finobj list
    ToBeFnz(usize),  // Sweeping tobefnz list
    Done,
}

impl SweepGc {
    fn is_done(&self) -> bool {
        matches!(self, SweepGc::Done)
    }
}
