use crate::{
    GcObjectKind, LuaFunction, LuaTable, LuaValue,
    lua_value::{CClosureFunction, LuaString, LuaUpvalue, LuaUserdata, RClosureFunction},
    lua_vm::LuaState,
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
/// Compressed to 8 bytes (was 16). Layout:
///
///   `packed: u32` — marked + index, bit layout:
///     bits  0-7 : `marked` — color + age + finalized bit
///       - Bits 0-2: Age (G_NEW=0 .. G_TOUCHED2=6)
///       - Bit 3: WHITE0
///       - Bit 4: WHITE1
///       - Bit 5: BLACK
///       - Bit 6: FINALIZEDBIT
///       - Bit 7: Reserved
///     bits 8-31 : `index` — position in GcList (24-bit, max ~16.7M objects)
///
///   `size: u32` — allocation-time memory size estimate (for GC pacing).
///     Set once at creation, never updated. This ensures consistent
///     accounting between trace_object (allocation) and sweep (deallocation).
///
/// **Tri-color invariant**: Gray is implicit - an object is gray iff it has no white bits AND no black bit.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct GcHeader {
    packed: u32,
    pub size: u32,
}

impl std::fmt::Debug for GcHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcHeader")
            .field("marked", &self.marked())
            .field("index", &self.index())
            .field("size", &self.size)
            .finish()
    }
}

impl GcHeader {
    const MARKED_MASK: u32 = 0xFF; // bits 0-7
    const INDEX_SHIFT: u32 = 8;
    const INDEX_MAX: u32 = (1 << 24) - 1; // 16,777,215

    // ============ Raw field access ============

    #[inline(always)]
    pub fn marked(&self) -> u8 {
        self.packed as u8
    }

    #[inline(always)]
    fn set_marked_bits(&mut self, m: u8) {
        self.packed = (self.packed & !Self::MARKED_MASK) | (m as u32);
    }

    #[inline(always)]
    pub fn index(&self) -> usize {
        (self.packed >> Self::INDEX_SHIFT) as usize
    }

    #[inline(always)]
    pub fn set_index(&mut self, idx: usize) {
        debug_assert!(
            idx <= Self::INDEX_MAX as usize,
            "GcList index overflow: {idx} > {}",
            Self::INDEX_MAX
        );
        self.packed = (self.packed & Self::MARKED_MASK) | ((idx as u32) << Self::INDEX_SHIFT);
    }
}

impl Default for GcHeader {
    fn default() -> Self {
        // WARNING: Default creates a GRAY object (no color bits set)
        // This is INCORRECT for new objects - they should be WHITE
        // Use GcHeader::with_white(current_white) instead when creating GC objects
        GcHeader {
            packed: G_NEW as u32,
            size: 0,
        }
    }
}

impl GcHeader {
    /// Create a new header with given white bit and age G_NEW, index=0
    ///
    /// **CRITICAL**: All new GC objects MUST use this constructor with current_white from GC
    #[inline(always)]
    pub fn with_white(current_white: u8) -> Self {
        debug_assert!(
            current_white == 0 || current_white == 1,
            "current_white must be 0 or 1"
        );
        GcHeader {
            packed: ((1 << (WHITE0BIT + current_white)) | G_NEW) as u32,
            size: 0,
        }
    }

    // ============ Age Operations (generational GC) ============

    /// Get object age (bits 0-2)
    #[inline(always)]
    pub fn age(&self) -> u8 {
        self.marked() & AGEBITS
    }

    /// Set object age (preserves color bits and index)
    #[inline(always)]
    pub fn set_age(&mut self, age: u8) {
        debug_assert!(age <= G_TOUCHED2, "Invalid age value");
        let m = (self.marked() & !AGEBITS) | (age & AGEBITS);
        self.set_marked_bits(m);
    }

    /// Check if object is old (age > G_SURVIVAL)
    #[inline(always)]
    pub fn is_old(&self) -> bool {
        self.age() > G_SURVIVAL
    }

    // ============ Color Operations (tri-color marking) ============

    #[inline(always)]
    pub fn is_white(&self) -> bool {
        (self.marked() & WHITEBITS) != 0
    }

    #[inline(always)]
    pub fn is_current_white(&self, current_white: u8) -> bool {
        debug_assert!(
            current_white == 0 || current_white == 1,
            "current_white must be 0 or 1"
        );
        (self.marked() & (1 << (WHITE0BIT + current_white))) != 0
    }

    #[inline(always)]
    pub fn is_black(&self) -> bool {
        (self.marked() & (1 << BLACKBIT)) != 0
    }

    #[inline(always)]
    pub fn is_gray(&self) -> bool {
        (self.marked() & (WHITEBITS | (1 << BLACKBIT))) == 0
    }

    // ============ Special Flags ============

    #[inline(always)]
    pub fn to_finalize(&self) -> bool {
        (self.marked() & (1 << FINALIZEDBIT)) != 0
    }

    #[inline(always)]
    pub fn set_finalized(&mut self) {
        self.set_marked_bits(self.marked() | (1 << FINALIZEDBIT));
    }

    #[inline(always)]
    pub fn clear_finalized(&mut self) {
        self.set_marked_bits(self.marked() & !(1 << FINALIZEDBIT));
    }

    // ============ Color Transitions ============

    #[inline(always)]
    pub fn make_white(&mut self, current_white: u8) {
        debug_assert!(
            current_white == 0 || current_white == 1,
            "current_white must be 0 or 1"
        );
        let m = (self.marked() & !MASKCOLORS) | (1 << (WHITE0BIT + current_white));
        self.set_marked_bits(m);
    }

    #[inline(always)]
    pub fn make_gray(&mut self) {
        self.set_marked_bits(self.marked() & !MASKCOLORS);
    }

    #[inline(always)]
    pub fn make_black(&mut self) {
        let m = (self.marked() & !WHITEBITS) | (1 << BLACKBIT);
        self.set_marked_bits(m);
    }

    #[inline(always)]
    pub fn nw2black(&mut self) {
        debug_assert!(!self.is_white(), "nw2black called on white object");
        self.set_marked_bits(self.marked() | (1 << BLACKBIT));
    }

    // ============ Death Detection ============

    #[inline(always)]
    pub fn is_dead(&self, other_white: u8) -> bool {
        debug_assert!(
            other_white == 0 || other_white == 1,
            "other_white must be 0 or 1"
        );
        (self.marked() & (1 << (WHITE0BIT + other_white))) != 0
    }

    #[inline(always)]
    pub fn otherwhite(current_white: u8) -> u8 {
        current_white ^ 1
    }

    #[inline(always)]
    pub fn change_white(&mut self) {
        self.set_marked_bits(self.marked() ^ WHITEBITS);
    }

    // ============ Generational GC Age Transitions ============

    #[inline(always)]
    pub fn make_old0(&mut self) {
        self.set_age(G_OLD0);
    }

    #[inline(always)]
    pub fn make_old1(&mut self) {
        self.set_age(G_OLD1);
    }

    #[inline(always)]
    pub fn make_old(&mut self) {
        self.set_age(G_OLD);
    }

    #[inline(always)]
    pub fn make_touched1(&mut self) {
        self.set_age(G_TOUCHED1);
    }

    #[inline(always)]
    pub fn make_touched2(&mut self) {
        self.set_age(G_TOUCHED2);
    }

    #[inline(always)]
    pub fn make_survival(&mut self) {
        self.set_age(G_SURVIVAL);
    }

    // ============ Utility Methods ============

    #[inline(always)]
    pub fn is_marked(&self) -> bool {
        !self.is_white()
    }
}

pub trait HasGcHeader {
    fn header(&self) -> &GcHeader;
}

#[repr(C)]
pub struct Gc<T> {
    pub header: GcHeader,
    pub data: T,
}

impl<T> Gc<T> {
    pub fn new(data: T, current_white: u8, size: u32) -> Self {
        let mut header = GcHeader::with_white(current_white);
        header.size = size;
        Gc { header, data }
    }
}

impl<T> HasGcHeader for Gc<T> {
    fn header(&self) -> &GcHeader {
        &self.header
    }
}

pub type GcString = Gc<LuaString>;
pub type GcBinary = Gc<Vec<u8>>;
pub type GcTable = Gc<LuaTable>;
pub type GcFunction = Gc<LuaFunction>;
pub type GcCClosure = Gc<CClosureFunction>;
pub type GcRClosure = Gc<RClosureFunction>;
pub type GcUpvalue = Gc<LuaUpvalue>;
pub type GcThread = Gc<LuaState>;
pub type GcUserdata = Gc<LuaUserdata>;

#[derive(Debug)]
pub struct GcPtr<T: HasGcHeader> {
    ptr: u64,
    _marker: std::marker::PhantomData<*const T>,
}

impl<T: HasGcHeader> std::hash::Hash for GcPtr<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ptr.hash(state);
    }
}

// Manual implementation of Clone and Copy to avoid trait bound requirements on T
// GcPtr is always Copy regardless of T, since it only stores a u64 pointer
impl<T: HasGcHeader> Clone for GcPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: HasGcHeader> Copy for GcPtr<T> {}

impl<T: HasGcHeader> Eq for GcPtr<T> {}

impl<T: HasGcHeader> PartialEq for GcPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl<T: HasGcHeader> GcPtr<T> {
    pub fn new(ptr: *const T) -> Self {
        Self {
            ptr: ptr as u64,
            _marker: std::marker::PhantomData,
        }
    }

    /// Construct from raw u64 (used by GcObjectPtr tagged-pointer unpacking)
    #[inline(always)]
    pub fn from_raw(raw: u64) -> Self {
        Self {
            ptr: raw,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn null() -> Self {
        Self {
            ptr: 0,
            _marker: std::marker::PhantomData,
        }
    }

    /// Get the raw u64 pointer value (used by GcObjectPtr tagged-pointer packing)
    #[inline(always)]
    pub fn as_u64(&self) -> u64 {
        self.ptr
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.ptr as *const T
    }

    #[inline(always)]
    pub fn as_mut_ptr(&self) -> *mut T {
        self.ptr as *mut T
    }

    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub fn as_mut_ref(&self) -> &mut T {
        unsafe { &mut *(self.as_mut_ptr()) }
    }

    #[inline(always)]
    pub fn as_ref(&self) -> &T {
        unsafe { &*(self.as_ptr()) }
    }

    pub fn is_null(&self) -> bool {
        self.ptr == 0
    }
}

pub type UpvaluePtr = GcPtr<GcUpvalue>;
pub type BinaryPtr = GcPtr<GcBinary>;
pub type TablePtr = GcPtr<GcTable>;
pub type StringPtr = GcPtr<GcString>;
pub type FunctionPtr = GcPtr<GcFunction>;
pub type CClosurePtr = GcPtr<GcCClosure>;
pub type RClosurePtr = GcPtr<GcRClosure>;
pub type UserdataPtr = GcPtr<GcUserdata>;
pub type ThreadPtr = GcPtr<GcThread>;

/// Compressed GcObjectPtr — tagged pointer in a single `u64` (8 bytes, was 16).
///
/// x86-64 user-space pointers use at most 48 bits. We store a 4-bit type tag
/// in bits 60-63, leaving bits 0-47 for the pointer. This is safe because:
/// - Windows user addresses < 0x0000_7FFF_FFFF_FFFF
/// - Linux user addresses < 0x0000_7FFF_FFFF_F000
/// - Tag values 0-8 in bits 60-63 never collide with valid addresses.
///
/// Because all `Gc<T>` are `#[repr(C)]` with `header: GcHeader` at offset 0,
/// `header()` / `header_mut()` are direct pointer casts — no match dispatch.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct GcObjectPtr(u64);

impl std::fmt::Debug for GcObjectPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GcObjectPtr({:?}, 0x{:012x})",
            self.kind(),
            self.raw_ptr()
        )
    }
}

impl std::hash::Hash for GcObjectPtr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl GcObjectPtr {
    const TAG_SHIFT: u32 = 60;
    const PTR_MASK: u64 = (1u64 << 48) - 1; // low 48 bits

    // Tag values — must match GcObjectKind repr(u8)
    const TAG_STRING: u64 = 0;
    const TAG_TABLE: u64 = 1;
    const TAG_FUNCTION: u64 = 2;
    const TAG_CCLOSURE: u64 = 3;
    const TAG_RCLOSURE: u64 = 4;
    const TAG_UPVALUE: u64 = 5;
    const TAG_THREAD: u64 = 6;
    const TAG_USERDATA: u64 = 7;
    const TAG_BINARY: u64 = 8;

    #[inline(always)]
    fn new_tagged(ptr: u64, tag: u64) -> Self {
        debug_assert!(
            ptr & !Self::PTR_MASK == 0,
            "pointer exceeds 48 bits: 0x{ptr:016x}"
        );
        Self(ptr | (tag << Self::TAG_SHIFT))
    }

    #[inline(always)]
    fn tag(&self) -> u8 {
        (self.0 >> Self::TAG_SHIFT) as u8
    }

    #[inline(always)]
    fn raw_ptr(&self) -> u64 {
        self.0 & Self::PTR_MASK
    }

    // ============ Header access — zero-cost via #[repr(C)] guarantee ============

    /// Access the GcHeader at offset 0 of the pointed-to Gc<T>.
    /// Safe because all Gc<T> are #[repr(C)] with header as first field.
    #[inline(always)]
    pub fn header(&self) -> Option<&GcHeader> {
        let p = self.raw_ptr();
        if p == 0 {
            None
        } else {
            Some(unsafe { &*(p as *const GcHeader) })
        }
    }

    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub fn header_mut(&self) -> Option<&mut GcHeader> {
        let p = self.raw_ptr();
        if p == 0 {
            None
        } else {
            Some(unsafe { &mut *(p as *mut GcHeader) })
        }
    }

    #[inline(always)]
    pub fn kind(&self) -> GcObjectKind {
        // Safety: tag values 0-8 match repr(u8) of GcObjectKind
        unsafe { std::mem::transmute(self.tag()) }
    }

    #[inline(always)]
    pub(crate) fn index(&self) -> usize {
        // Reads index directly from header
        self.header().map(|h| h.index()).unwrap_or(0)
    }

    pub fn fix_gc_object(&mut self) {
        if let Some(header) = self.header_mut() {
            header.set_age(G_OLD);
            header.make_gray(); // Gray forever, like Lua 5.5
        }
    }

    // ============ Typed pointer extraction ============

    #[inline(always)]
    pub fn as_string_ptr(&self) -> StringPtr {
        debug_assert!(self.tag() == Self::TAG_STRING as u8);
        StringPtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_table_ptr(&self) -> TablePtr {
        debug_assert!(self.tag() == Self::TAG_TABLE as u8);
        TablePtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_function_ptr(&self) -> FunctionPtr {
        debug_assert!(self.tag() == Self::TAG_FUNCTION as u8);
        FunctionPtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_cclosure_ptr(&self) -> CClosurePtr {
        debug_assert!(self.tag() == Self::TAG_CCLOSURE as u8);
        CClosurePtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_rclosure_ptr(&self) -> RClosurePtr {
        debug_assert!(self.tag() == Self::TAG_RCLOSURE as u8);
        RClosurePtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_upvalue_ptr(&self) -> UpvaluePtr {
        debug_assert!(self.tag() == Self::TAG_UPVALUE as u8);
        UpvaluePtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_thread_ptr(&self) -> ThreadPtr {
        debug_assert!(self.tag() == Self::TAG_THREAD as u8);
        ThreadPtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_userdata_ptr(&self) -> UserdataPtr {
        debug_assert!(self.tag() == Self::TAG_USERDATA as u8);
        UserdataPtr::from_raw(self.raw_ptr())
    }

    #[inline(always)]
    pub fn as_binary_ptr(&self) -> BinaryPtr {
        debug_assert!(self.tag() == Self::TAG_BINARY as u8);
        BinaryPtr::from_raw(self.raw_ptr())
    }

    // ============ Pattern matching helpers (for code that still uses if-let) ============

    #[inline(always)]
    pub fn is_string(&self) -> bool {
        self.tag() == Self::TAG_STRING as u8
    }

    #[inline(always)]
    pub fn is_table(&self) -> bool {
        self.tag() == Self::TAG_TABLE as u8
    }

    #[inline(always)]
    pub fn is_upvalue(&self) -> bool {
        self.tag() == Self::TAG_UPVALUE as u8
    }

    #[inline(always)]
    pub fn is_thread(&self) -> bool {
        self.tag() == Self::TAG_THREAD as u8
    }

    #[inline(always)]
    pub fn is_function(&self) -> bool {
        self.tag() == Self::TAG_FUNCTION as u8
    }

    #[inline(always)]
    pub fn is_cclosure(&self) -> bool {
        self.tag() == Self::TAG_CCLOSURE as u8
    }

    #[inline(always)]
    pub fn is_rclosure(&self) -> bool {
        self.tag() == Self::TAG_RCLOSURE as u8
    }

    #[inline(always)]
    pub fn is_userdata(&self) -> bool {
        self.tag() == Self::TAG_USERDATA as u8
    }

    #[inline(always)]
    pub fn is_binary(&self) -> bool {
        self.tag() == Self::TAG_BINARY as u8
    }
}

impl From<StringPtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: StringPtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_STRING)
    }
}

impl From<BinaryPtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: BinaryPtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_BINARY)
    }
}

impl From<TablePtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: TablePtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_TABLE)
    }
}

impl From<FunctionPtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: FunctionPtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_FUNCTION)
    }
}

impl From<UpvaluePtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: UpvaluePtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_UPVALUE)
    }
}

impl From<ThreadPtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: ThreadPtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_THREAD)
    }
}

impl From<UserdataPtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: UserdataPtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_USERDATA)
    }
}

impl From<CClosurePtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: CClosurePtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_CCLOSURE)
    }
}

impl From<RClosurePtr> for GcObjectPtr {
    #[inline(always)]
    fn from(ptr: RClosurePtr) -> Self {
        Self::new_tagged(ptr.as_u64(), Self::TAG_RCLOSURE)
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
    CClosure(Box<GcCClosure>),
    RClosure(Box<GcRClosure>),
    Binary(Box<GcBinary>),
}

impl GcObjectOwner {
    /// Compute the approximate memory size of this object (replaces the old
    /// `header.size` field). Called at allocation/deallocation for GC pacing —
    /// NOT on hot paths, so the match + dynamic calculation is fine.
    pub fn compute_size(&self) -> usize {
        match self {
            GcObjectOwner::String(s) => std::mem::size_of::<GcString>() + s.data.str.len(),
            GcObjectOwner::Binary(b) => std::mem::size_of::<GcBinary>() + b.data.len(),
            GcObjectOwner::Table(t) => {
                let base = std::mem::size_of::<GcTable>();
                let asize = t.data.impl_table.asize as usize;
                let array_bytes = if asize > 0 { asize * 17 + 4 } else { 0 };
                let hash_bytes = {
                    let hs = t.data.hash_size();
                    if hs > 0 { hs * 24 + 8 } else { 0 }
                };
                base + array_bytes + hash_bytes
            }
            GcObjectOwner::Function(f) => {
                f.data.chunk().proto_data_size as usize + std::mem::size_of_val(f.data.upvalues())
            }
            GcObjectOwner::CClosure(c) => {
                std::mem::size_of::<GcCClosure>()
                    + c.data.upvalues().len() * std::mem::size_of::<LuaValue>()
            }
            GcObjectOwner::RClosure(r) => {
                std::mem::size_of::<GcRClosure>()
                    + r.data.upvalues().len() * std::mem::size_of::<LuaValue>()
            }
            GcObjectOwner::Upvalue(_) => 64, // fixed estimate
            GcObjectOwner::Thread(t) => std::mem::size_of::<GcThread>() + t.data.stack.len() * 16,
            GcObjectOwner::Userdata(_) => std::mem::size_of::<GcUserdata>(),
        }
    }

    /// Return the stored allocation-time size (from header.size)
    #[inline]
    pub fn size(&self) -> usize {
        self.header().size as usize
    }

    pub fn header(&self) -> &GcHeader {
        (match self {
            GcObjectOwner::String(s) => &s.header,
            GcObjectOwner::Table(t) => &t.header,
            GcObjectOwner::Function(f) => &f.header,
            GcObjectOwner::CClosure(c) => &c.header,
            GcObjectOwner::RClosure(r) => &r.header,
            GcObjectOwner::Upvalue(u) => &u.header,
            GcObjectOwner::Thread(t) => &t.header,
            GcObjectOwner::Userdata(u) => &u.header,
            GcObjectOwner::Binary(b) => &b.header,
        }) as _
    }

    pub fn header_mut(&mut self) -> &mut GcHeader {
        (match self {
            GcObjectOwner::String(s) => &mut s.header,
            GcObjectOwner::Table(t) => &mut t.header,
            GcObjectOwner::Function(f) => &mut f.header,
            GcObjectOwner::CClosure(c) => &mut c.header,
            GcObjectOwner::RClosure(r) => &mut r.header,
            GcObjectOwner::Upvalue(u) => &mut u.header,
            GcObjectOwner::Thread(t) => &mut t.header,
            GcObjectOwner::Userdata(u) => &mut u.header,
            GcObjectOwner::Binary(b) => &mut b.header,
        }) as _
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

    pub fn as_closure_ptr(&self) -> Option<CClosurePtr> {
        match self {
            GcObjectOwner::CClosure(c) => Some(CClosurePtr::new(c.as_ref() as *const GcCClosure)),
            _ => None,
        }
    }

    pub fn as_rclosure_ptr(&self) -> Option<RClosurePtr> {
        match self {
            GcObjectOwner::RClosure(r) => Some(RClosurePtr::new(r.as_ref() as *const GcRClosure)),
            _ => None,
        }
    }

    pub fn as_gc_ptr(&self) -> GcObjectPtr {
        match self {
            GcObjectOwner::String(s) => {
                GcObjectPtr::from(StringPtr::new(s.as_ref() as *const GcString))
            }
            GcObjectOwner::Table(t) => {
                GcObjectPtr::from(TablePtr::new(t.as_ref() as *const GcTable))
            }
            GcObjectOwner::Function(f) => {
                GcObjectPtr::from(FunctionPtr::new(f.as_ref() as *const GcFunction))
            }
            GcObjectOwner::Upvalue(u) => {
                GcObjectPtr::from(UpvaluePtr::new(u.as_ref() as *const GcUpvalue))
            }
            GcObjectOwner::Thread(t) => {
                GcObjectPtr::from(ThreadPtr::new(t.as_ref() as *const GcThread))
            }
            GcObjectOwner::Userdata(u) => {
                GcObjectPtr::from(UserdataPtr::new(u.as_ref() as *const GcUserdata))
            }
            GcObjectOwner::Binary(b) => {
                GcObjectPtr::from(BinaryPtr::new(b.as_ref() as *const GcBinary))
            }
            GcObjectOwner::CClosure(c) => {
                GcObjectPtr::from(CClosurePtr::new(c.as_ref() as *const GcCClosure))
            }
            GcObjectOwner::RClosure(r) => {
                GcObjectPtr::from(RClosurePtr::new(r.as_ref() as *const GcRClosure))
            }
        }
    }

    pub fn as_table_mut(&mut self) -> Option<&mut LuaTable> {
        match self {
            GcObjectOwner::Table(t) => Some(&mut t.data),
            _ => None,
        }
    }

    pub fn as_function_mut(&mut self) -> Option<&mut LuaFunction> {
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

    pub fn as_cclosure_mut(&mut self) -> Option<&mut CClosureFunction> {
        match self {
            GcObjectOwner::CClosure(c) => Some(&mut c.data),
            _ => None,
        }
    }

    pub fn as_rclosure_mut(&mut self) -> Option<&mut RClosureFunction> {
        match self {
            GcObjectOwner::RClosure(r) => Some(&mut r.data),
            _ => None,
        }
    }

    pub fn size_of_data(&self) -> usize {
        self.header().size as usize
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
}

impl GcList {
    #[inline]
    pub fn new() -> Self {
        Self {
            gc_list: Vec::new(),
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            gc_list: Vec::with_capacity(cap),
        }
    }

    /// Allocate a new object and return a GcPtr to it
    /// O(1) allocation: push to Vec, track index in header, return pointer to Box contents
    #[inline]
    pub fn add(&mut self, mut value: GcObjectOwner) {
        let index = self.gc_list.len();
        value.header_mut().set_index(index);
        self.gc_list.push(value);
    }

    /// Free an object using its pointer
    /// O(1) via swap_remove: moves last object to removed position, updates its index
    #[inline]
    pub fn remove(&mut self, gc_ptr: GcObjectPtr) -> GcObjectOwner {
        let index = gc_ptr.index();
        let last_index = self.gc_list.len() - 1;
        if index != last_index {
            // Update moved object's index
            let moved_obj = &mut self.gc_list[last_index];
            moved_obj.header_mut().set_index(index);
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

    pub fn iter_ptrs(&self) -> impl Iterator<Item = GcObjectPtr> + '_ {
        self.gc_list.iter().map(|obj| obj.as_gc_ptr())
    }

    /// Check if an object is in this list by checking its index
    /// O(1) check using the object's stored index
    #[inline]
    pub fn contains(&self, gc_ptr: GcObjectPtr) -> bool {
        let index = gc_ptr.index();
        if index < self.gc_list.len() {
            // Verify it's actually the same object (not just same index)
            self.gc_list[index].as_gc_ptr() == gc_ptr
        } else {
            false
        }
    }

    /// Try to remove an object, returning Some(owner) if found, None otherwise
    #[inline]
    pub fn try_remove(&mut self, gc_ptr: GcObjectPtr) -> Option<GcObjectOwner> {
        if self.contains(gc_ptr) {
            Some(self.remove(gc_ptr))
        } else {
            None
        }
    }

    /// Get GcObjectOwner by index (for iteration with ownership)
    /// This method panics if index is out of bounds
    #[inline]
    pub fn get_owner(&self, index: usize) -> &GcObjectOwner {
        &self.gc_list[index]
    }

    /// Take all objects out and return as Vec, leaving self empty
    #[inline]
    pub fn take_all(&mut self) -> Vec<GcObjectOwner> {
        std::mem::take(&mut self.gc_list)
    }

    /// Add multiple objects (used when moving between generation lists)
    #[inline]
    pub fn add_all(&mut self, objects: Vec<GcObjectOwner>) {
        for mut obj in objects {
            let index = self.gc_list.len();
            obj.header_mut().set_index(index);
            self.gc_list.push(obj);
        }
    }
}

impl Default for GcList {
    fn default() -> Self {
        Self::new()
    }
}
