// ============ GC Header ============
use std::rc::Rc;

use crate::{
    Chunk, GcObjectKind, LuaTable,
    lua_value::{LuaString, LuaUpvalue, LuaUserdata},
    lua_vm::{CFunction, LuaState},
};

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
    pub marked: u8,   // Color and age bits combined
    pub size: u32,    // Size of the object in bytes (for memory tracking)
    pub index: usize, // Index in the GC pool
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
            index: 0,
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
            index: 0,
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
}

pub struct Gc<T> {
    pub header: GcHeader,
    pub data: T,
}

impl<T> Gc<T> {
    pub fn new(data: T, current_white: u8, size: u32) -> Self {
        Gc {
            header: GcHeader::with_white(current_white, size),
            data,
        }
    }
}

pub type GcString = Gc<LuaString>;
pub type GcTable = Gc<LuaTable>;
pub type GcFunction = Gc<FunctionBody>;
pub type GcUpvalue = Gc<LuaUpvalue>;
pub type GcThread = Gc<LuaState>;
pub type GcUserdata = Gc<LuaUserdata>;
pub type GcBinary = Gc<Vec<u8>>;

#[derive(Debug)]
pub struct GcPtr<T> {
    ptr: u64,
    _marker: std::marker::PhantomData<*const T>,
}

impl<T> std::hash::Hash for GcPtr<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ptr.hash(state);
    }
}

// Manual implementation of Clone and Copy to avoid trait bound requirements on T
// GcPtr is always Copy regardless of T, since it only stores a u64 pointer
impl<T> Clone for GcPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for GcPtr<T> {}

impl<T> Eq for GcPtr<T> {}

impl<T> PartialEq for GcPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GcObjectPtr {
    String(StringPtr),
    Table(TablePtr),
    Function(FunctionPtr),
    Upvalue(UpvaluePtr),
    Thread(ThreadPtr),
    Userdata(UserdataPtr),
    Binary(BinaryPtr),
}

impl GcObjectPtr {
    pub fn header(&self) -> Option<&GcHeader> {
        match self {
            GcObjectPtr::String(p) => Some(&p.as_ref().header),
            GcObjectPtr::Table(p) => Some(&p.as_ref().header),
            GcObjectPtr::Function(p) => Some(&p.as_ref().header),
            GcObjectPtr::Upvalue(p) => Some(&p.as_ref().header),
            GcObjectPtr::Thread(p) => Some(&p.as_ref().header),
            GcObjectPtr::Userdata(p) => Some(&p.as_ref().header),
            GcObjectPtr::Binary(p) => Some(&p.as_ref().header),
        }
    }

    pub fn header_mut(&self) -> Option<&mut GcHeader> {
        match self {
            GcObjectPtr::String(p) => Some(&mut p.as_mut_ref().header),
            GcObjectPtr::Table(p) => Some(&mut p.as_mut_ref().header),
            GcObjectPtr::Function(p) => Some(&mut p.as_mut_ref().header),
            GcObjectPtr::Upvalue(p) => Some(&mut p.as_mut_ref().header),
            GcObjectPtr::Thread(p) => Some(&mut p.as_mut_ref().header),
            GcObjectPtr::Userdata(p) => Some(&mut p.as_mut_ref().header),
            GcObjectPtr::Binary(p) => Some(&mut p.as_mut_ref().header),
        }
    }

    pub fn kind(&self) -> GcObjectKind {
        match self {
            GcObjectPtr::String(_) => GcObjectKind::String,
            GcObjectPtr::Table(_) => GcObjectKind::Table,
            GcObjectPtr::Function(_) => GcObjectKind::Function,
            GcObjectPtr::Upvalue(_) => GcObjectKind::Upvalue,
            GcObjectPtr::Thread(_) => GcObjectKind::Thread,
            GcObjectPtr::Userdata(_) => GcObjectKind::Userdata,
            GcObjectPtr::Binary(_) => GcObjectKind::Binary,
        }
    }

    pub(crate) fn index(&self) -> usize {
        match self {
            GcObjectPtr::String(p) => p.as_ref().header.index,
            GcObjectPtr::Table(p) => p.as_ref().header.index,
            GcObjectPtr::Function(p) => p.as_ref().header.index,
            GcObjectPtr::Upvalue(p) => p.as_ref().header.index,
            GcObjectPtr::Thread(p) => p.as_ref().header.index,
            GcObjectPtr::Userdata(p) => p.as_ref().header.index,
            GcObjectPtr::Binary(p) => p.as_ref().header.index,
        }
    }

    pub fn fix_gc_object(&mut self) {
        if let Some(header) = self.header_mut() {
            header.set_age(G_OLD);
            header.make_gray(); // Gray forever, like Lua 5.5
        }
    }
}

impl From<StringPtr> for GcObjectPtr {
    fn from(ptr: StringPtr) -> Self {
        GcObjectPtr::String(ptr)
    }
}

impl From<BinaryPtr> for GcObjectPtr {
    fn from(ptr: BinaryPtr) -> Self {
        GcObjectPtr::Binary(ptr)
    }
}

impl From<TablePtr> for GcObjectPtr {
    fn from(ptr: TablePtr) -> Self {
        GcObjectPtr::Table(ptr)
    }
}

impl From<FunctionPtr> for GcObjectPtr {
    fn from(ptr: FunctionPtr) -> Self {
        GcObjectPtr::Function(ptr)
    }
}

impl From<UpvaluePtr> for GcObjectPtr {
    fn from(ptr: UpvaluePtr) -> Self {
        GcObjectPtr::Upvalue(ptr)
    }
}

impl From<ThreadPtr> for GcObjectPtr {
    fn from(ptr: ThreadPtr) -> Self {
        GcObjectPtr::Thread(ptr)
    }
}

impl From<UserdataPtr> for GcObjectPtr {
    fn from(ptr: UserdataPtr) -> Self {
        GcObjectPtr::Userdata(ptr)
    }
}

// ============ GC-managed Objects ============
pub enum GcObjectOwner {
    String(Box<GcString>),
    Table(Box<GcTable>),
    Function(Box<GcFunction>),
    Upvalue(Box<GcUpvalue>),
    Thread(Box<GcThread>),
    Userdata(Box<GcUserdata>),
    Binary(Box<GcBinary>),
}

impl GcObjectOwner {
    pub fn size(&self) -> usize {
        self.header().size as usize
    }

    pub fn header(&self) -> &GcHeader {
        match self {
            GcObjectOwner::String(s) => &s.header,
            GcObjectOwner::Table(t) => &t.header,
            GcObjectOwner::Function(f) => &f.header,
            GcObjectOwner::Upvalue(u) => &u.header,
            GcObjectOwner::Thread(t) => &t.header,
            GcObjectOwner::Userdata(u) => &u.header,
            GcObjectOwner::Binary(b) => &b.header,
        }
    }

    pub fn header_mut(&mut self) -> &mut GcHeader {
        match self {
            GcObjectOwner::String(s) => &mut s.header,
            GcObjectOwner::Table(t) => &mut t.header,
            GcObjectOwner::Function(f) => &mut f.header,
            GcObjectOwner::Upvalue(u) => &mut u.header,
            GcObjectOwner::Thread(t) => &mut t.header,
            GcObjectOwner::Userdata(u) => &mut u.header,
            GcObjectOwner::Binary(b) => &mut b.header,
        }
    }

    /// Get type tag of this object
    #[inline(always)]
    pub fn as_str_ptr(&self) -> Option<StringPtr> {
        match self {
            GcObjectOwner::String(s) => Some(StringPtr::new(s.as_ref() as *const GcString)),
            _ => None,
        }
    }

    pub fn as_table_ptr(&self) -> Option<TablePtr> {
        match self {
            GcObjectOwner::Table(t) => Some(TablePtr::new(t.as_ref() as *const GcTable)),
            _ => None,
        }
    }

    pub fn as_function_ptr(&self) -> Option<FunctionPtr> {
        match self {
            GcObjectOwner::Function(f) => Some(FunctionPtr::new(f.as_ref() as *const GcFunction)),
            _ => None,
        }
    }

    pub fn as_upvalue_ptr(&self) -> Option<UpvaluePtr> {
        match self {
            GcObjectOwner::Upvalue(u) => Some(UpvaluePtr::new(u.as_ref() as *const GcUpvalue)),
            _ => None,
        }
    }

    pub fn as_thread_ptr(&self) -> Option<ThreadPtr> {
        match self {
            GcObjectOwner::Thread(t) => Some(ThreadPtr::new(t.as_ref() as *const GcThread)),
            _ => None,
        }
    }

    pub fn as_userdata_ptr(&self) -> Option<UserdataPtr> {
        match self {
            GcObjectOwner::Userdata(u) => Some(UserdataPtr::new(u.as_ref() as *const GcUserdata)),
            _ => None,
        }
    }

    pub fn as_binary_ptr(&self) -> Option<BinaryPtr> {
        match self {
            GcObjectOwner::Binary(b) => Some(BinaryPtr::new(b.as_ref() as *const GcBinary)),
            _ => None,
        }
    }

    pub fn as_gc_ptr(&self) -> GcObjectPtr {
        match self {
            GcObjectOwner::String(s) => {
                GcObjectPtr::String(StringPtr::new(s.as_ref() as *const GcString))
            }
            GcObjectOwner::Table(t) => {
                GcObjectPtr::Table(TablePtr::new(t.as_ref() as *const GcTable))
            }
            GcObjectOwner::Function(f) => {
                GcObjectPtr::Function(FunctionPtr::new(f.as_ref() as *const GcFunction))
            }
            GcObjectOwner::Upvalue(u) => {
                GcObjectPtr::Upvalue(UpvaluePtr::new(u.as_ref() as *const GcUpvalue))
            }
            GcObjectOwner::Thread(t) => {
                GcObjectPtr::Thread(ThreadPtr::new(t.as_ref() as *const GcThread))
            }
            GcObjectOwner::Userdata(u) => {
                GcObjectPtr::Userdata(UserdataPtr::new(u.as_ref() as *const GcUserdata))
            }
            GcObjectOwner::Binary(b) => {
                GcObjectPtr::Binary(BinaryPtr::new(b.as_ref() as *const GcBinary))
            }
        }
    }

    pub fn as_table_mut(&mut self) -> Option<&mut LuaTable> {
        match self {
            GcObjectOwner::Table(t) => Some(&mut t.data),
            _ => None,
        }
    }

    pub fn as_function_mut(&mut self) -> Option<&mut FunctionBody> {
        match self {
            GcObjectOwner::Function(f) => Some(&mut f.data),
            _ => None,
        }
    }

    pub fn as_upvalue_mut(&mut self) -> Option<&mut LuaUpvalue> {
        match self {
            GcObjectOwner::Upvalue(u) => Some(&mut u.data),
            _ => None,
        }
    }

    pub fn as_thread_mut(&mut self) -> Option<&mut LuaState> {
        match self {
            GcObjectOwner::Thread(t) => Some(&mut t.data),
            _ => None,
        }
    }

    pub fn as_userdata_mut(&mut self) -> Option<&mut LuaUserdata> {
        match self {
            GcObjectOwner::Userdata(u) => Some(&mut u.data),
            _ => None,
        }
    }

    pub fn size_of_data(&self) -> usize {
        self.header().size as usize
    }
}

/// Function body - either Lua bytecode or C function
pub enum FunctionBody {
    /// Lua function with bytecode chunk
    /// Now includes cached upvalue pointers for direct access (zero-overhead like Lua C)
    Lua(Rc<Chunk>, Vec<UpvaluePtr>),
    /// C function (native Rust function) with cached upvalues
    CClosure(CFunction, Vec<UpvaluePtr>),
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
    pub fn upvalues(&self) -> &Vec<UpvaluePtr> {
        match &self {
            FunctionBody::CClosure(_, uv) => uv,
            FunctionBody::Lua(_, uv) => uv,
        }
    }

    /// Get mutable access to cached upvalues for updating pointers
    #[inline(always)]
    pub fn upvalues_mut(&mut self) -> &mut Vec<UpvaluePtr> {
        match self {
            FunctionBody::CClosure(_, uv) => uv,
            FunctionBody::Lua(_, uv) => uv,
        }
    }
}

/// High-performance Vec-based pool for GC objects
/// - O(1) allocation: direct push to Vec, returns GcPtr
/// - O(1) deallocation: swap_remove using tracked pool_index  
/// - O(live_objects) iteration: always compact, no holes!
/// - No free_list needed: objects are truly removed via swap_remove
/// - GcPtr-based: external references use pointers, not indices
pub struct GcList {
    gc_list: Vec<GcObjectOwner>,
    fixed_list: Vec<GcObjectOwner>,
}

impl GcList {
    #[inline]
    pub fn new() -> Self {
        Self {
            gc_list: Vec::new(),
            fixed_list: Vec::new(),
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            gc_list: Vec::with_capacity(cap),
            fixed_list: Vec::new(),
        }
    }

    /// Allocate a new object and return a GcPtr to it
    /// O(1) allocation: push to Vec, track index in header, return pointer to Box contents
    #[inline]
    pub fn alloc(&mut self, mut value: GcObjectOwner) {
        let index = self.gc_list.len();
        value.header_mut().index = index;
        self.gc_list.push(value);
    }

    /// Free an object using its pointer
    /// O(1) via swap_remove: moves last object to removed position, updates its index
    #[inline]
    pub fn free(&mut self, gc_ptr: GcObjectPtr) -> GcObjectOwner {
        let index = gc_ptr.index();
        let last_index = self.gc_list.len() - 1;
        if index != last_index {
            // Update moved object's index
            let moved_obj = &mut self.gc_list[last_index];
            moved_obj.header_mut().index = index;
        }

        // swap_remove: O(1) removal by moving last element to this position
        self.gc_list.swap_remove(index)
    }

    /// Current number of live objects (always equals Vec length, no holes!)
    #[inline]
    pub fn len(&self) -> usize {
        self.gc_list.len()
    }

    /// Check if pool is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.gc_list.is_empty()
    }

    /// Iterate over all live objects (always compact, O(live_objects))
    pub fn iter(&self) -> impl Iterator<Item = &GcObjectOwner> + '_ {
        self.gc_list.iter()
    }

    /// Iterate over all live objects mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut GcObjectOwner> + '_ {
        self.gc_list.iter_mut()
    }

    /// Shrink internal storage to fit current objects
    pub fn shrink_to_fit(&mut self) {
        self.gc_list.shrink_to_fit();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        self.gc_list.clear();
    }

    /// Get Vec capacity (for diagnostics)
    #[inline]
    pub fn capacity(&self) -> usize {
        self.gc_list.capacity()
    }

    pub fn get(&self, index: usize) -> Option<&GcObjectOwner> {
        self.gc_list.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut GcObjectOwner> {
        self.gc_list.get_mut(index)
    }

    pub fn fixed(&mut self, gc_ptr: GcObjectPtr) {
        if let Some(header) = gc_ptr.header_mut() {
            header.set_age(G_OLD);
            header.make_gray(); // Gray forever, like Lua 5.5
        }

        let gc_owner = self.free(gc_ptr);
        self.fixed_list.push(gc_owner);
    }
}

impl Default for GcList {
    fn default() -> Self {
        Self::new()
    }
}
