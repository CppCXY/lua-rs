// ============ GC Header ============
use std::rc::Rc;

use crate::{
    lua_value::LuaUserdata, lua_vm::{CFunction, LuaState}, Chunk, GcId, LuaTable, LuaValue
};

/// Cached upvalue - stores both ID and direct pointer for fast access
/// Mimics Lua C's cl->upvals[i] which is a direct pointer to UpValue
#[derive(Clone, Copy, Debug)]
pub struct CachedUpvalue {
    /// Direct pointer to the Upvalue object for fast access
    /// SAFETY: This pointer is valid as long as the Upvalue exists in ObjectPool
    /// The Upvalue is kept alive by the GC as long as this function exists
    pub ptr: UpvaluePtr,
}

impl CachedUpvalue {
    #[inline(always)]
    pub fn new(ptr: UpvaluePtr) -> Self {
        Self { ptr }
    }

    /// Get the upvalue value directly through the cached pointer
    /// SAFETY: Caller must ensure the pointer is still valid
    /// current_thread: The thread attempting to read the upvalue (used for stack access optimization)
    #[inline(always)]
    pub fn get_value(&self, l: &LuaState) -> LuaValue {
        unsafe {
            let upval = &*self.ptr.as_ptr();
            match upval.data {
                Upvalue::Open(stack_index) => {
                    // Directly access the stack value
                    l.stack()[stack_index].clone()
                }
                Upvalue::Closed(val) => val.clone(),
            }
        }
    }

    #[inline(always)]
    pub fn set_value(&self, l: &mut LuaState, value: LuaValue) {
        unsafe {
            let upval = &mut *self.ptr.as_mut_ptr();
            match &mut upval.data {
                Upvalue::Open(stack_index) => {
                    // Set value directly on the stack
                    l.stack_mut()[*stack_index] = value;
                }
                Upvalue::Closed(val) => {
                    // Update closed value
                    *val = value;
                }
            }
        }
    }
}

// ============ GC Constants (from Lua 5.5 lgc.h) ============
// Object ages for generational GC
// Uses 3 bits (0-7) - stored in bits 0-2 of marked field
pub const G_NEW: u8 = 0; // Created in current cycle
pub const G_SURVIVAL: u8 = 1; // Created in previous cycle (survived one minor)
pub const G_OLD0: u8 = 2; // Marked old by forward barrier in this cycle
pub const G_OLD1: u8 = 3; // First full cycle as old
pub const G_OLD: u8 = 4; // Really old object (not to be visited in minor)
pub const G_TOUCHED1: u8 = 5; // Old object touched this cycle
pub const G_TOUCHED2: u8 = 6; // Old object touched in previous cycle

// Color bit positions in marked field
pub const WHITE0BIT: u8 = 3; // Object is white (type 0)
pub const WHITE1BIT: u8 = 4; // Object is white (type 1)
pub const BLACKBIT: u8 = 5; // Object is black
pub const FINALIZEDBIT: u8 = 6; // Object has been marked for finalization

// Bit masks
pub const WHITEBITS: u8 = (1 << WHITE0BIT) | (1 << WHITE1BIT);
pub const AGEBITS: u8 = 0x07; // Mask for age bits (bits 0-2: 0b00000111)
pub const MASKCOLORS: u8 = (1 << BLACKBIT) | WHITEBITS;
pub const MASKGCBITS: u8 = MASKCOLORS | AGEBITS;

/// GC object header - embedded in every GC-managed object
/// Port of Lua 5.5's CommonHeader (lgc.h)
///
/// Bit layout of `marked` field:
/// - Bits 0-2: Age (G_NEW=0, G_SURVIVAL=1, G_OLD0=2, G_OLD1=3, G_OLD=4, G_TOUCHED1=5, G_TOUCHED2=6)
/// - Bit 3: WHITE0 (white type 0)
/// - Bit 4: WHITE1 (white type 1)  
/// - Bit 5: BLACK (fully marked)
/// - Bit 6: FINALIZEDBIT (marked for finalization)
/// - Bit 7: Reserved for future use
///
/// **Tri-color invariant**: Gray is implicit - an object is gray iff it has no white bits AND no black bit.
/// This allows gray detection without an explicit gray bit: `!is_white() && !is_black()`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct GcHeader {
    pub marked: u8, // Color and age bits combined
    pub size: u32,  // Size of the object in bytes (for memory tracking)
}

impl Default for GcHeader {
    fn default() -> Self {
        // WARNING: Default creates a GRAY object (no color bits set)
        // This is INCORRECT for new objects - they should be WHITE
        // Use GcHeader::with_white(current_white) instead when creating GC objects
        // Port of lgc.c: New objects MUST be created with luaC_white(g)
        GcHeader {
            marked: G_NEW, // Age 0, no color bits set (gray state - WRONG for new objects!)
            size: 0,
        }
    }
}

impl GcHeader {
    /// Create a new header with given white bit and age G_NEW
    /// Port of lgc.c: luaC_white(g) which returns (currentwhite & WHITEBITS)
    /// combined with makewhite(g,x) which sets white color for new objects
    ///
    /// **CRITICAL**: All new GC objects MUST use this constructor with current_white from GC
    /// Using Default::default() creates incorrect GRAY objects that may be prematurely collected
    #[inline(always)]
    pub fn with_white(current_white: u8, size: u32) -> Self {
        debug_assert!(
            current_white == 0 || current_white == 1,
            "current_white must be 0 or 1"
        );
        GcHeader {
            marked: (1 << (WHITE0BIT + current_white)) | G_NEW,
            size,
        }
    }

    // ============ Age Operations (generational GC) ============

    /// Get object age (bits 0-2)
    /// Port of lgc.h: getage(o) returns (o->marked & AGEBITS)
    #[inline(always)]
    pub fn age(&self) -> u8 {
        self.marked & AGEBITS
    }

    /// Set object age (preserves color bits)
    /// Port of lgc.h: setage(o,a) sets age while preserving other bits
    #[inline(always)]
    pub fn set_age(&mut self, age: u8) {
        debug_assert!(age <= G_TOUCHED2, "Invalid age value");
        self.marked = (self.marked & !AGEBITS) | (age & AGEBITS);
    }

    /// Check if object is old (age > G_SURVIVAL)
    /// Port of lgc.h: isold(o) macro
    #[inline(always)]
    pub fn is_old(&self) -> bool {
        self.age() > G_SURVIVAL
    }

    // ============ Color Operations (tri-color marking) ============

    /// Check if object is white (either WHITE0 or WHITE1)
    /// Port of lgc.h: iswhite(x) macro
    #[inline(always)]
    pub fn is_white(&self) -> bool {
        (self.marked & WHITEBITS) != 0
    }

    /// Check if object is black
    /// Port of lgc.h: isblack(x) macro
    #[inline(always)]
    pub fn is_black(&self) -> bool {
        (self.marked & (1 << BLACKBIT)) != 0
    }

    /// Check if object is gray (neither white nor black)
    /// Port of lgc.h: isgray(x) macro
    /// Gray objects are in gray lists waiting to be scanned
    #[inline(always)]
    pub fn is_gray(&self) -> bool {
        (self.marked & (WHITEBITS | (1 << BLACKBIT))) == 0
    }

    // ============ Special Flags ============

    /// Check if object is marked for finalization
    /// Port of lgc.h: tofinalize(x) macro
    #[inline(always)]
    pub fn to_finalize(&self) -> bool {
        (self.marked & (1 << FINALIZEDBIT)) != 0
    }

    /// Mark object for finalization
    #[inline(always)]
    pub fn set_finalized(&mut self) {
        self.marked |= 1 << FINALIZEDBIT;
    }

    /// Clear finalization mark
    #[inline(always)]
    pub fn clear_finalized(&mut self) {
        self.marked &= !(1 << FINALIZEDBIT);
    }

    /// Check if object is fixed (never collected)
    /// In Lua 5.5, fixed objects also use FINALIZEDBIT (bit 6) but never sweep them
    /// Port of lgc.h: isold(x) but for permanent objects
    #[inline(always)]
    pub fn is_fixed(&self) -> bool {
        // In Lua 5.5, fixed strings and permanent objects have special age G_OLD
        // and are never collected. We can use same bit as finalized since
        // fixed objects won't be finalized.
        self.age() == G_OLD && self.to_finalize()
    }

    /// Mark object as fixed (never collected)
    /// Port of lgc.h: luaC_fix()
    #[inline(always)]
    pub fn set_fixed(&mut self) {
        self.set_age(G_OLD);
        self.set_finalized();
    }

    // ============ Color Transitions ============

    /// Make object white with given current_white (0 or 1)
    /// Port of lgc.c: makewhite(g,x) macro
    /// Sets object to current white color, preserving age
    #[inline(always)]
    pub fn make_white(&mut self, current_white: u8) {
        debug_assert!(
            current_white == 0 || current_white == 1,
            "current_white must be 0 or 1"
        );
        // Clear all color bits, then set the appropriate white bit
        self.marked = (self.marked & !MASKCOLORS) | (1 << (WHITE0BIT + current_white));
    }

    /// Make object gray (clear all color bits, keep age)
    /// Port of lgc.c: set2gray(x) macro
    /// Gray objects are in gray lists waiting to be scanned
    #[inline(always)]
    pub fn make_gray(&mut self) {
        self.marked &= !MASKCOLORS; // Clear color bits, preserve age
    }

    /// Make object black (from any color)
    /// Port of lgc.c: set2black(x) macro
    /// Black objects are fully marked (object and all references scanned)
    #[inline(always)]
    pub fn make_black(&mut self) {
        self.marked = (self.marked & !WHITEBITS) | (1 << BLACKBIT);
    }

    /// Make object black from non-white state (assertion version)
    /// Port of lgc.c: nw2black(x) macro
    #[inline(always)]
    pub fn nw2black(&mut self) {
        debug_assert!(!self.is_white(), "nw2black called on white object");
        self.marked |= 1 << BLACKBIT;
    }

    // ============ Death Detection ============

    /// Check if object is dead (has the "other" white bit set)
    /// Port of lgc.h: isdead(g,v) and isdeadm(ow,m) macros
    /// During sweep, objects with "other white" are garbage
    #[inline(always)]
    pub fn is_dead(&self, other_white: u8) -> bool {
        debug_assert!(
            other_white == 0 || other_white == 1,
            "other_white must be 0 or 1"
        );
        (self.marked & (1 << (WHITE0BIT + other_white))) != 0
    }

    /// Get the "other white" bit from current white
    /// Port of lgc.h: otherwhite(g) macro returns (currentwhite ^ WHITEBITS)
    #[inline(always)]
    pub fn otherwhite(current_white: u8) -> u8 {
        current_white ^ 1
    }

    /// Change white type (flip between WHITE0 and WHITE1)
    /// Port of lgc.h: changewhite(x) macro
    #[inline(always)]
    pub fn change_white(&mut self) {
        self.marked ^= WHITEBITS;
    }

    // ============ Generational GC Age Transitions ============

    /// Advance object to OLD0 (marked old by forward barrier)
    #[inline(always)]
    pub fn make_old0(&mut self) {
        self.set_age(G_OLD0);
    }

    /// Advance object to OLD1 (first full cycle as old)
    #[inline(always)]
    pub fn make_old1(&mut self) {
        self.set_age(G_OLD1);
    }

    /// Advance object to fully OLD (won't be visited in minor collections)
    #[inline(always)]
    pub fn make_old(&mut self) {
        self.set_age(G_OLD);
    }

    /// Mark object as TOUCHED1 (old object modified in this cycle)
    #[inline(always)]
    pub fn make_touched1(&mut self) {
        self.set_age(G_TOUCHED1);
    }

    /// Mark object as TOUCHED2 (old object modified in previous cycle)
    #[inline(always)]
    pub fn make_touched2(&mut self) {
        self.set_age(G_TOUCHED2);
    }

    /// Make object SURVIVAL (survived one minor collection)
    #[inline(always)]
    pub fn make_survival(&mut self) {
        self.set_age(G_SURVIVAL);
    }

    // ============ Utility Methods ============

    /// Check if object is marked (not white)
    /// Convenience method for readability
    #[inline(always)]
    pub fn is_marked(&self) -> bool {
        !self.is_white()
    }

    /// Legacy method for backward compatibility
    #[deprecated(note = "Use make_black/make_white directly for clarity")]
    #[inline(always)]
    pub fn set_marked(&mut self, marked: bool) {
        if marked {
            self.make_black();
        } else {
            self.make_white(0);
        }
    }
}

pub struct Gc<T> {
    pub header: GcHeader,
    pub data: T,
}

impl <T> Gc<T> {
    pub fn new(data: T, current_white: u8, size: u32) -> Self {
        Gc {
            header: GcHeader::with_white(current_white, size),
            data,
        }
    }
}

pub type GcString = Gc<String>;
pub type GcTable = Gc<LuaTable>;
pub type GcFunction = Gc<FunctionBody>;
pub type GcUpvalue = Gc<Upvalue>;
pub type GcThread = Gc<LuaState>;
pub type GcUserdata = Gc<LuaUserdata>;
pub type GcBinary = Gc<Vec<u8>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GcPtr<T>{
    ptr: u64,
    _marker: std::marker::PhantomData<*const T>,
}

impl<T> GcPtr<T> {
    pub fn new(ptr: *const T) -> Self {
        Self {
            ptr: ptr as u64,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn null() -> Self {
        Self {
            ptr: 0,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn as_ptr(&self) -> *const T {
        self.ptr as *const T
    }

    pub fn as_mut_ptr(&self) -> *mut T {
        self.ptr as *mut T
    }

    pub fn as_mut_ref(&self) -> &mut T {
        unsafe { &mut *(self.as_mut_ptr()) }
    }

    pub fn as_ref(&self) -> &T {
        unsafe { &*(self.as_ptr()) }
    }

    pub fn is_null(&self) -> bool {
        self.ptr == 0
    }
}

pub type UpvaluePtr = GcPtr<GcUpvalue>;
pub type TablePtr = GcPtr<GcTable>;
pub type StringPtr = GcPtr<GcString>;
pub type FunctionPtr = GcPtr<GcFunction>;
pub type BinaryPtr = GcPtr<GcBinary>;
pub type UserdataPtr = GcPtr<GcUserdata>;
pub type ThreadPtr = GcPtr<GcThread>;

// ============ GC-managed Objects ============
pub enum GcObject {
    String(Box<GcString>),
    Table(Box<GcTable>),
    Function(Box<GcFunction>),
    Upvalue(Box<GcUpvalue>),
    Thread(Box<GcThread>),
    Userdata(Box<GcUserdata>),
    Binary(Box<GcBinary>),
}

impl GcObject {
    pub fn size(&self) -> usize {
        self.header().size as usize
    }

    pub fn header(&self) -> &GcHeader {
        match self {
            GcObject::String(s) => &s.header,
            GcObject::Table(t) => &t.header,
            GcObject::Function(f) => &f.header,
            GcObject::Upvalue(u) => &u.header,
            GcObject::Thread(t) => &t.header,
            GcObject::Userdata(u) => &u.header,
            GcObject::Binary(b) => &b.header,
        }
    }
    
    pub fn header_mut(&mut self) -> &mut GcHeader {
        match self {
            GcObject::String(s) => &mut s.header,
            GcObject::Table(t) => &mut t.header,
            GcObject::Function(f) => &mut f.header,
            GcObject::Upvalue(u) => &mut u.header,
            GcObject::Thread(t) => &mut t.header,
            GcObject::Userdata(u) => &mut u.header,
            GcObject::Binary(b) => &mut b.header,
        }
    }

    /// Get type tag of this object
    #[inline(always)]
    pub fn as_str_ptr(&self) -> Option<StringPtr> {
        match self {
            GcObject::String(s) => Some(StringPtr::new(s.as_ref() as *const GcString)),
            _ => None,
        }
    }

    pub fn as_table_ptr(&self) -> Option<TablePtr> {
        match self {
            GcObject::Table(t) => Some(TablePtr::new(t.as_ref() as *const GcTable)),
            _ => None,
        }
    }

    pub fn as_function_ptr(&self) -> Option<FunctionPtr> {
        match self {
            GcObject::Function(f) => Some(FunctionPtr::new(f.as_ref() as *const GcFunction)),
            _ => None,
        }
    }

    pub fn as_upvalue_ptr(&self) -> Option<UpvaluePtr> {
        match self {
            GcObject::Upvalue(u) => Some(UpvaluePtr::new(u.as_ref() as *const GcUpvalue)),
            _ => None,
        }
    }

    pub fn as_thread_ptr(&self) -> Option<ThreadPtr> {
        match self {
            GcObject::Thread(t) => Some(ThreadPtr::new(t.as_ref() as *const GcThread)),
            _ => None,
        }
    }

    pub fn as_userdata_ptr(&self) -> Option<UserdataPtr> {
        match self {
            GcObject::Userdata(u) => Some(UserdataPtr::new(u.as_ref() as *const GcUserdata)),
            _ => None,
        }
    }

    pub fn as_binary_ptr(&self) -> Option<BinaryPtr> {
        match self {
            GcObject::Binary(b) => Some(BinaryPtr::new(b.as_ref() as *const GcBinary)),
            _ => None,
        }
    }

    pub fn as_table_mut(&mut self) -> Option<&mut LuaTable> {
        match self {
            GcObject::Table(t) => Some(&mut t.data),
            _ => None,
        }
    }

    pub fn as_function_mut(&mut self) -> Option<&mut FunctionBody> {
        match self {
            GcObject::Function(f) => Some(&mut f.data),
            _ => None,
        }
    }

    pub fn as_upvalue_mut(&mut self) -> Option<&mut Upvalue> {
        match self {
            GcObject::Upvalue(u) => Some(&mut u.data),
            _ => None,
        }
    }

    pub fn as_thread_mut(&mut self) -> Option<&mut LuaState> {
        match self {
            GcObject::Thread(t) => Some(&mut t.data),
            _ => None,
        }
    }

    pub fn as_userdata_mut(&mut self) -> Option<&mut LuaUserdata> {
        match self {
            GcObject::Userdata(u) => Some(&mut u.data),
            _ => None,
        }
    }
}

/// Function body - either Lua bytecode or C function
pub enum FunctionBody {
    /// Lua function with bytecode chunk
    /// Now includes cached upvalue pointers for direct access (zero-overhead like Lua C)
    Lua(Rc<Chunk>, Vec<CachedUpvalue>),
    /// C function (native Rust function) with cached upvalues
    CClosure(CFunction, Vec<CachedUpvalue>),
}

impl FunctionBody {
    /// Check if this is a C function (any C variant)
    #[inline(always)]
    pub fn is_c_function(&self) -> bool {
        matches!(self, FunctionBody::CClosure(_, _))
    }

    /// Check if this is a Lua function
    #[inline(always)]
    pub fn is_lua_function(&self) -> bool {
        matches!(self, FunctionBody::Lua(_, _))
    }

    /// Get the chunk if this is a Lua function
    #[inline(always)]
    pub fn chunk(&self) -> Option<&Rc<Chunk>> {
        match &self {
            FunctionBody::Lua(chunk, _) => Some(chunk),
            _ => None,
        }
    }

    /// Get the C function pointer if this is any C function variant
    #[inline(always)]
    pub fn c_function(&self) -> Option<CFunction> {
        match &self {
            FunctionBody::CClosure(f, _) => Some(*f),
            FunctionBody::Lua(_, _) => None,
        }
    }

    /// Get cached upvalues (direct pointers for fast access)
    #[inline(always)]
    pub fn cached_upvalues(&self) -> &Vec<CachedUpvalue> {
        match &self {
            FunctionBody::CClosure(_, uv) => uv,
            FunctionBody::Lua(_, uv) => uv,
        }
    }

    /// Get mutable access to cached upvalues for updating pointers
    #[inline(always)]
    pub fn cached_upvalues_mut(&mut self) -> &mut Vec<CachedUpvalue> {
        match self {
            FunctionBody::CClosure(_, uv) => uv,
            FunctionBody::Lua(_, uv) => uv,
        }
    }
}

/// Upvalue with embedded GC header
///
/// Mimics Lua 5.5's UpVal structure with v.p pointer optimization:
/// - v.p always points to the actual value location (stack or u.value)
/// - When open: v.p points to stack[stack_index]
/// - When closed: v.p points to closed_value
///
/// This eliminates the branch in get/set operations, matching Lua C performance
pub enum Upvalue {
    Open(usize),
    Closed(LuaValue),
}

impl Upvalue {
    /// Check if this upvalue points to the given absolute stack index
    #[inline]
    pub fn points_to_index(&self, index: usize) -> bool {
        match self {
            Upvalue::Open(i) => *i == index,
            Upvalue::Closed(_) => false,
        }
    }

    /// Check if this upvalue is open (still points to stack)
    #[inline]
    pub fn is_open(&self) -> bool {
        matches!(self, Upvalue::Open(_))
    }

    /// Close this upvalue with the given value
    #[inline]
    pub fn close(&mut self, value: LuaValue) {
        *self = Upvalue::Closed(value);
    }

    /// Get the value of a closed upvalue (returns None if still open)
    #[inline]
    pub fn get_closed_value(&self) -> Option<LuaValue> {
        match self {
            Upvalue::Closed(val) => Some(val.clone()),
            Upvalue::Open(_) => None,
        }
    }

    /// Get the absolute stack index if this upvalue is open
    #[inline]
    pub fn get_stack_index(&self) -> Option<usize> {
        match self {
            Upvalue::Open(i) => Some(*i),
            Upvalue::Closed(_) => None,
        }
    }
}

/// Vec-based pool with free list for GC objects
/// - O(1) allocation and lookup (direct indexing)
/// - O(vec.len()) iteration (includes None slots, but fast for allocation-heavy workloads)
/// - Free list for ID reuse
/// - Optimized for fast allocation at the cost of slower iteration when sparse
pub struct GcPool {
    gc_list: Vec<Option<GcObject>>,
    free_list: Vec<u32>,
    count: usize,
}

impl GcPool {
    #[inline]
    pub fn new() -> Self {
        Self {
            gc_list: Vec::new(),
            free_list: Vec::new(),
            count: 0,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            gc_list: Vec::with_capacity(cap),
            free_list: Vec::with_capacity(cap / 8),
            count: 0,
        }
    }

    /// Allocate a new object and return its ID
    /// Super-fast O(1) allocation via Vec indexing
    #[inline]
    pub fn alloc(&mut self, value: GcObject) {
        self.count += 1;
        self.gc_list.push(Some(value));
    }

    /// Get immutable reference by ID
    #[inline(always)]
    pub fn get(&self, id: u32) -> Option<&GcObject> {
        self.gc_list.get(id as usize).and_then(|opt| opt.as_ref())
    }

    /// Get mutable reference by ID
    #[inline(always)]
    pub fn get_mut(&mut self, id: u32) -> Option<&mut GcObject> {
        self.gc_list
            .get_mut(id as usize)
            .and_then(|opt| opt.as_mut())
    }

    /// Free a slot (mark for reuse)
    #[inline]
    pub fn free(&mut self, id: u32) -> usize {
        if let Some(slot) = self.gc_list.get_mut(id as usize) {
            if slot.is_some() {
                let size = slot.as_ref().unwrap().size();
                *slot = None;
                self.free_list.push(id);
                self.count -= 1;
                return size;
            }
        }
        0
    }

    /// Get number of free slots in the free list
    #[inline]
    pub fn free_slots_count(&self) -> usize {
        self.free_list.len()
    }

    /// Trim trailing None values from the pool to reduce iteration overhead
    pub fn trim_tail(&mut self) {
        while self.gc_list.last().map_or(false, |v| v.is_none()) {
            self.gc_list.pop();
        }
        let max_valid = self.gc_list.len() as u32;
        self.free_list.retain(|&id| id < max_valid);
    }

    /// Check if a slot is occupied
    #[inline(always)]
    pub fn is_valid(&self, id: u32) -> bool {
        self.gc_list
            .get(id as usize)
            .map(|opt| opt.is_some())
            .unwrap_or(false)
    }

    /// Current number of live objects
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Length of free_list (recycled IDs)
    #[inline]
    pub fn free_list_len(&self) -> usize {
        self.free_list.len()
    }

    /// Total capacity of the gc_list Vec (including empty slots)
    #[inline]
    pub fn capacity(&self) -> usize {
        self.gc_list.len()
    }

    /// Iterate over all live objects
    pub fn iter(&self) -> impl Iterator<Item = (GcId, &GcObject)> + '_ {
        self.gc_list
            .iter()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_ref().map(|v| (v.trans_to_gcid(id as u32), v)))
    }

    /// Iterate over all live objects mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (GcId, &mut GcObject)> + '_ {
        self.gc_list
            .iter_mut()
            .enumerate()
            .filter_map(|(id, opt)| opt.as_mut().map(|v| (v.trans_to_gcid(id as u32), v)))
    }

    /// Shrink internal storage
    pub fn shrink_to_fit(&mut self) {
        self.gc_list.shrink_to_fit();
        self.free_list.shrink_to_fit();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        self.gc_list.clear();
        self.free_list.clear();
        self.count = 0;
    }
}

impl Default for GcPool {
    fn default() -> Self {
        Self::new()
    }
}
