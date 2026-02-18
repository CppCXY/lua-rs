// LuaValue - Lua 5.5 TValue implementation in Rust
//
// **完全按照Lua 5.5标准设计**
// 参考: lua-5.5.0/src/lobject.h
//
// Lua 5.5 TValue结构:
// ```c
// typedef union Value {
//   struct GCObject *gc;    /* collectable objects */
//   void *p;                /* light userdata */
//   lua_CFunction f;        /* light C functions */
//   lua_Integer i;          /* integer numbers */
//   lua_Number n;           /* float numbers */
// } Value;
//
// typedef struct TValue {
//   Value value_;
//   lu_byte tt_;            /* type tag */
// } TValue;
// ```
//
// Rust实现:
// - Value union: 8字节 (存储i64/f64/指针/GC对象ID)
// - tt_: 1字节 type tag
// - 总大小: 16字节 (带padding对齐)
//
// Type tag编码 (bits 0-5):
// - Bits 0-3: 基础类型 (LUA_TNIL, LUA_TBOOLEAN, LUA_TNUMBER, etc.)
// - Bits 4-5: variant bits (区分子类型,如integer/float, short/long string)
// - Bit 6: BIT_ISCOLLECTABLE (标记是否是GC对象)
use crate::lua_value::{CClosureFunction, LuaUserdata};
use crate::lua_vm::{CFunction, LuaState};
use crate::{
    BinaryPtr, CClosurePtr, FunctionPtr, GcBinary, GcCClosure, GcFunction, GcObjectPtr, GcString,
    GcTable, GcThread, GcUserdata, LuaFunction, LuaTable, StringPtr, TablePtr, ThreadPtr,
    UserdataPtr,
};

// ============ Basic type tags (bits 0-3) ============
// From lua.h
pub const LUA_TNIL: u8 = 0;
pub const LUA_TBOOLEAN: u8 = 1;
pub const LUA_TLIGHTUSERDATA: u8 = 2;
pub const LUA_TNUMBER: u8 = 3;
pub const LUA_TSTRING: u8 = 4;
pub const LUA_TTABLE: u8 = 5;
pub const LUA_TFUNCTION: u8 = 6;
pub const LUA_TUSERDATA: u8 = 7;
pub const LUA_TTHREAD: u8 = 8;

// ============ Variant tags (with bits 4-5) ============
macro_rules! makevariant {
    ($base:expr, $variant:expr) => {
        $base | ($variant << 4)
    };
}

// Nil variants
pub const LUA_VNIL: u8 = makevariant!(LUA_TNIL, 0);
pub const LUA_VEMPTY: u8 = makevariant!(LUA_TNIL, 1); // empty slot in table
pub const LUA_VABSTKEY: u8 = makevariant!(LUA_TNIL, 2); // absent key in table

// Boolean variants
pub const LUA_VFALSE: u8 = makevariant!(LUA_TBOOLEAN, 0);
pub const LUA_VTRUE: u8 = makevariant!(LUA_TBOOLEAN, 1);

// Number variants
pub const LUA_VNUMINT: u8 = makevariant!(LUA_TNUMBER, 0); // integer
pub const LUA_VNUMFLT: u8 = makevariant!(LUA_TNUMBER, 1); // float

// Light userdata (NOT collectable)
pub const LUA_VLIGHTUSERDATA: u8 = makevariant!(LUA_TLIGHTUSERDATA, 0);

// Collectable types (bit 6 set)
pub const BIT_ISCOLLECTABLE: u8 = 1 << 6;

// String variants (like Lua 5.5: LUA_VSHRSTR and LUA_VLNGSTR)
pub const LUA_VSHRSTR: u8 = LUA_TSTRING | BIT_ISCOLLECTABLE; // 0x44 - short string (interned)
pub const LUA_VLNGSTR: u8 = makevariant!(LUA_TSTRING, 1) | BIT_ISCOLLECTABLE; // 0x54 - long string (not interned)
pub const LUA_VBINARY: u8 = makevariant!(LUA_TSTRING, 2) | BIT_ISCOLLECTABLE; // 0x64 - binary data

// Table
pub const LUA_VTABLE: u8 = LUA_TTABLE | BIT_ISCOLLECTABLE; // 0x45

// Function variants
pub const LUA_VFUNCTION: u8 = makevariant!(LUA_TFUNCTION, 0) | BIT_ISCOLLECTABLE; // 0x46
pub const LUA_CCLOSURE: u8 = makevariant!(LUA_TFUNCTION, 1) | BIT_ISCOLLECTABLE; // 0x56 - C closure
pub const LUA_VLCF: u8 = makevariant!(LUA_TFUNCTION, 2); // 0x26 - light C function

// userdata and thread
pub const LUA_VUSERDATA: u8 = LUA_TUSERDATA | BIT_ISCOLLECTABLE; // 0x47
pub const LUA_VTHREAD: u8 = LUA_TTHREAD | BIT_ISCOLLECTABLE; // 0x48

#[inline(always)]
pub const fn novariant(tt: u8) -> u8 {
    tt & 0x0F
}

#[allow(unused)]
#[inline(always)]
pub const fn withvariant(tt: u8) -> u8 {
    tt & 0x3F
}

// ============ Value union ============
/// Lua 5.5 Value union (8 bytes)
/// Now stores raw pointers for GC objects for direct access
#[derive(Clone, Copy)]
#[repr(C)]
pub union Value {
    pub ptr: *const u8,           // GC object pointer (stable via Box in Gc<T>)
    pub p: *mut std::ffi::c_void, // light userdata pointer
    pub f: usize,                 // light C function pointer (converted from fn pointer)
    pub i: i64,                   // integer number
    pub n: f64,                   // float number
}

impl Value {
    #[inline(always)]
    pub const fn nil() -> Self {
        Value { i: 0 }
    }

    #[inline(always)]
    pub const fn integer(i: i64) -> Self {
        Value { i }
    }

    #[inline(always)]
    pub fn float(n: f64) -> Self {
        Value { n }
    }

    #[inline(always)]
    pub fn lightuserdata(p: *mut std::ffi::c_void) -> Self {
        Value { p }
    }

    #[inline(always)]
    pub fn cfunction(f: CFunction) -> Self {
        Value { f: f as usize }
    }
}

// ============ TValue ============
/// Lua 5.5 TValue structure (16 bytes)
/// Now with embedded GC ID for direct pointer access
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuaValue {
    pub(crate) value: Value, // 8 bytes: value or pointer
    pub(crate) tt: u8,       // 1 byte: type tag
}

impl LuaValue {
    // ============ Type Tag Access ============

    /// Get type tag (for compatibility)
    #[inline(always)]
    pub fn tt(&self) -> u8 {
        self.tt
    }

    // ============ Constructors ============

    #[inline(always)]
    pub const fn nil() -> Self {
        Self {
            value: Value::nil(),
            tt: LUA_VNIL,
        }
    }

    #[inline(always)]
    pub const fn empty() -> Self {
        Self {
            value: Value::nil(),
            tt: LUA_VEMPTY,
        }
    }

    #[inline(always)]
    pub const fn abstkey() -> Self {
        Self {
            value: Value::nil(),
            tt: LUA_VABSTKEY,
        }
    }

    #[inline(always)]
    pub const fn boolean(b: bool) -> Self {
        Self {
            value: Value::nil(),
            tt: if b { LUA_VTRUE } else { LUA_VFALSE },
        }
    }

    #[inline(always)]
    pub const fn integer(i: i64) -> Self {
        Self {
            value: Value::integer(i),
            tt: LUA_VNUMINT,
        }
    }

    #[inline(always)]
    pub fn float(n: f64) -> Self {
        Self {
            value: Value::float(n),
            tt: LUA_VNUMFLT,
        }
    }

    #[inline(always)]
    pub fn number(n: f64) -> Self {
        Self::float(n)
    }

    // ============ In-place mutators (for VM performance) ============

    /// Create a short string value (interned, <= 40 bytes)
    #[inline(always)]
    pub fn shortstring(ptr: StringPtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_VSHRSTR,
        }
    }

    /// Create a long string value (not interned, > 40 bytes)
    #[inline(always)]
    pub fn longstring(ptr: StringPtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_VLNGSTR,
        }
    }

    #[inline(always)]
    pub fn binary(ptr: BinaryPtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_VBINARY,
        }
    }

    #[inline(always)]
    pub fn table(ptr: TablePtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_VTABLE,
        }
    }

    #[inline(always)]
    pub fn function(ptr: FunctionPtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_VFUNCTION,
        }
    }

    // Light C function (NOT collectable)
    #[inline(always)]
    pub fn cfunction(f: CFunction) -> Self {
        Self {
            value: Value::cfunction(f),
            tt: LUA_VLCF,
        }
    }

    #[inline(always)]
    pub fn cclosure(ptr: CClosurePtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_CCLOSURE,
        }
    }

    #[inline(always)]
    pub fn lightuserdata(p: *mut std::ffi::c_void) -> Self {
        Self {
            value: Value::lightuserdata(p),
            tt: LUA_VLIGHTUSERDATA,
        }
    }

    #[inline(always)]
    pub fn userdata(ptr: UserdataPtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_VUSERDATA,
        }
    }

    #[inline(always)]
    pub fn thread(ptr: ThreadPtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            },
            tt: LUA_VTHREAD,
        }
    }

    // ============ Type checking (following Lua 5.5 macros) ============

    /// ttype(o) - type without variants (bits 0-3)
    #[inline(always)]
    pub(crate) fn ttype(&self) -> u8 {
        novariant(self.tt())
    }

    #[allow(unused)]
    /// ttypetag(o) - type tag with variants (bits 0-5)
    #[inline(always)]
    pub(crate) fn ttypetag(&self) -> u8 {
        withvariant(self.tt())
    }

    /// checktag(o, t) - exact tag match
    #[inline(always)]
    pub(crate) fn checktag(&self, t: u8) -> bool {
        self.tt() == t
    }

    /// checktype(o, t) - type match (ignoring variants)
    #[inline(always)]
    pub(crate) fn checktype(&self, t: u8) -> bool {
        novariant(self.tt()) == t
    }

    /// iscollectable(o) - is this a GC object?
    #[inline(always)]
    pub(crate) fn iscollectable(&self) -> bool {
        (self.tt() & BIT_ISCOLLECTABLE) != 0
    }

    /// is_collectable - alias for Rust naming convention
    #[inline(always)]
    pub(crate) fn is_collectable(&self) -> bool {
        self.iscollectable()
    }

    // Specific type checks
    #[inline(always)]
    pub(crate) fn ttisnil(&self) -> bool {
        self.checktype(LUA_TNIL)
    }

    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn ttisstrictnil(&self) -> bool {
        self.checktag(LUA_VNIL)
    }

    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn ttisempty(&self) -> bool {
        self.checktag(LUA_VEMPTY)
    }

    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn isabstkey(&self) -> bool {
        self.checktag(LUA_VABSTKEY)
    }

    #[inline(always)]
    pub(crate) fn ttisboolean(&self) -> bool {
        self.checktype(LUA_TBOOLEAN)
    }

    #[inline(always)]
    pub(crate) fn ttisfalse(&self) -> bool {
        self.checktag(LUA_VFALSE)
    }

    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn ttistrue(&self) -> bool {
        self.checktag(LUA_VTRUE)
    }

    #[inline(always)]
    pub(crate) fn ttisnumber(&self) -> bool {
        self.checktype(LUA_TNUMBER)
    }

    #[inline(always)]
    pub(crate) fn ttisinteger(&self) -> bool {
        self.checktag(LUA_VNUMINT)
    }

    #[inline(always)]
    pub(crate) fn ttisfloat(&self) -> bool {
        self.checktag(LUA_VNUMFLT)
    }

    #[inline(always)]
    pub(crate) fn ttisstring(&self) -> bool {
        self.checktype(LUA_TSTRING)
    }

    #[inline(always)]
    pub(crate) fn ttisbinary(&self) -> bool {
        self.checktag(LUA_VBINARY)
    }

    #[inline(always)]
    pub(crate) fn ttistable(&self) -> bool {
        self.checktag(LUA_VTABLE)
    }

    #[inline(always)]
    pub(crate) fn ttisfunction(&self) -> bool {
        self.checktype(LUA_TFUNCTION)
    }

    #[inline(always)]
    pub(crate) fn ttisluafunction(&self) -> bool {
        self.checktag(LUA_VFUNCTION)
    }

    #[inline(always)]
    pub(crate) fn ttiscfunction(&self) -> bool {
        self.checktag(LUA_VLCF)
    }
    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn ttislightuserdata(&self) -> bool {
        self.checktag(LUA_VLIGHTUSERDATA)
    }

    #[inline(always)]
    pub(crate) fn ttisfulluserdata(&self) -> bool {
        self.checktag(LUA_VUSERDATA)
    }

    #[inline(always)]
    pub(crate) fn ttisthread(&self) -> bool {
        self.checktag(LUA_VTHREAD)
    }

    // ============ Value extraction ============

    #[inline(always)]
    pub(crate) fn bvalue(&self) -> bool {
        debug_assert!(self.ttisboolean());
        self.tt() == LUA_VTRUE
    }

    #[inline(always)]
    pub(crate) fn ivalue(&self) -> i64 {
        debug_assert!(self.ttisinteger());
        unsafe { self.value.i }
    }

    #[inline(always)]
    pub(crate) fn fltvalue(&self) -> f64 {
        debug_assert!(self.ttisfloat());
        unsafe { self.value.n }
    }

    /// nvalue - convert any number to f64
    #[inline(always)]
    pub(crate) fn nvalue(&self) -> f64 {
        debug_assert!(self.ttisnumber());
        if self.ttisinteger() {
            unsafe { self.value.i as f64 }
        } else {
            unsafe { self.value.n }
        }
    }

    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn pvalue(&self) -> *mut std::ffi::c_void {
        debug_assert!(self.ttislightuserdata());
        unsafe { self.value.p }
    }

    #[inline(always)]
    pub(crate) fn fvalue(&self) -> CFunction {
        debug_assert!(self.ttiscfunction());
        unsafe { std::mem::transmute(self.value.f) }
    }

    // ============ pub API ============

    #[inline(always)]
    pub fn is_nil(&self) -> bool {
        self.ttisnil()
    }

    #[inline(always)]
    pub fn is_boolean(&self) -> bool {
        self.ttisboolean()
    }

    #[inline(always)]
    pub fn is_integer(&self) -> bool {
        self.ttisinteger()
    }

    #[inline(always)]
    pub fn is_float(&self) -> bool {
        self.ttisfloat()
    }

    #[inline(always)]
    pub fn is_number(&self) -> bool {
        self.ttisnumber()
    }

    #[inline(always)]
    pub fn is_string(&self) -> bool {
        self.ttisstring()
    }

    #[inline(always)]
    pub fn is_short_string(&self) -> bool {
        self.checktag(LUA_VSHRSTR)
    }

    #[inline(always)]
    pub fn is_binary(&self) -> bool {
        self.ttisbinary()
    }

    #[inline(always)]
    pub fn is_table(&self) -> bool {
        self.ttistable()
    }

    #[inline(always)]
    pub fn is_function(&self) -> bool {
        self.ttisfunction()
    }

    #[inline(always)]
    pub fn is_lua_function(&self) -> bool {
        self.ttisluafunction()
    }

    #[inline(always)]
    pub fn is_cfunction(&self) -> bool {
        self.ttiscfunction()
    }

    #[inline(always)]
    pub fn is_cclosure(&self) -> bool {
        self.checktag(LUA_CCLOSURE)
    }

    #[inline(always)]
    pub fn is_c_callable(&self) -> bool {
        self.is_cfunction() || self.is_cclosure()
    }

    #[inline(always)]
    pub fn is_userdata(&self) -> bool {
        self.ttisfulluserdata()
    }

    #[inline(always)]
    pub fn is_thread(&self) -> bool {
        self.ttisthread()
    }

    #[inline(always)]
    pub fn is_callable(&self) -> bool {
        self.ttisfunction()
    }

    #[inline(always)]
    pub fn as_boolean(&self) -> Option<bool> {
        if self.ttisboolean() {
            Some(self.bvalue())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_bool(&self) -> Option<bool> {
        self.as_boolean()
    }

    #[inline(always)]
    pub fn as_integer_strict(&self) -> Option<i64> {
        if self.ttisinteger() {
            Some(self.ivalue())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_integer(&self) -> Option<i64> {
        if self.ttisinteger() {
            Some(self.ivalue())
        } else if self.ttisfloat() {
            // Lua 5.4+ semantics: floats with zero fraction are integers
            // Use proper range check matching C Lua's lua_numbertointeger:
            // f must be in [i64::MIN, -(i64::MIN as f64)) since i64::MAX as f64
            // rounds up to 2^63 which is NOT representable as i64.
            let f = self.fltvalue();
            if f >= (i64::MIN as f64) && f < -(i64::MIN as f64) && f == (f as i64 as f64) {
                Some(f as i64)
            } else {
                None
            }
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_float(&self) -> Option<f64> {
        if self.ttisfloat() {
            Some(self.fltvalue())
        } else if self.ttisinteger() {
            Some(self.ivalue() as f64)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_number(&self) -> Option<f64> {
        if self.ttisnumber() {
            Some(self.nvalue())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_str(&self) -> Option<&str> {
        // String type but not binary
        if self.ttisstring() && !self.ttisbinary() {
            Some(unsafe {
                let ptr = self.value.ptr;
                let s: &GcString = &*(ptr as *const GcString);
                &s.data.as_str()
            })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_binary(&self) -> Option<&[u8]> {
        if self.ttisbinary() {
            Some(unsafe {
                let ptr = self.value.ptr;
                let v: &GcBinary = &*(ptr as *const GcBinary);
                &v.data.as_slice()
            })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_table(&self) -> Option<&LuaTable> {
        if self.ttistable() {
            let v = unsafe { &*(self.value.ptr as *const GcTable) };
            Some(&v.data)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_table_mut(&self) -> Option<&mut LuaTable> {
        if self.ttistable() {
            let v = unsafe { &mut *(self.value.ptr as *mut GcTable) };
            Some(&mut v.data)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_lua_function(&self) -> Option<&LuaFunction> {
        if self.ttisluafunction() {
            let func = unsafe { &*(self.value.ptr as *const GcFunction) };
            Some(&func.data)
        } else {
            None
        }
    }

    /// Unsafe version that skips the type tag check.
    /// Caller MUST ensure `self.is_lua_function()` is true.
    #[inline(always)]
    pub unsafe fn as_lua_function_unchecked(&self) -> &LuaFunction {
        debug_assert!(self.ttisluafunction());
        let func = unsafe { &*(self.value.ptr as *const GcFunction) };
        &func.data
    }

    #[inline(always)]
    pub fn as_lua_function_mut(&self) -> Option<&mut LuaFunction> {
        if self.ttisluafunction() {
            let func = unsafe { &mut *(self.value.ptr as *mut GcFunction) };
            Some(&mut func.data)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_cfunction(&self) -> Option<CFunction> {
        if self.ttiscfunction() {
            Some(self.fvalue())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_cclosure(&self) -> Option<&CClosureFunction> {
        if self.is_cclosure() {
            let gc = unsafe { &*(self.value.ptr as *const GcCClosure) };
            Some(&gc.data)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_cclosure_mut(&self) -> Option<&mut CClosureFunction> {
        if self.is_cclosure() {
            let gc = unsafe { &mut *(self.value.ptr as *mut GcCClosure) };
            Some(&mut gc.data)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_userdata_mut(&self) -> Option<&mut LuaUserdata> {
        if self.ttisfulluserdata() {
            let gc = unsafe { &mut *(self.value.ptr as *mut GcUserdata) };
            Some(&mut gc.data)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_thread_mut(&self) -> Option<&mut LuaState> {
        if self.ttisthread() {
            let v = unsafe { &mut *(self.value.ptr as *mut GcThread) };
            Some(&mut v.data)
        } else {
            None
        }
    }

    pub fn as_table_ptr(&self) -> Option<TablePtr> {
        if self.ttistable() {
            Some(TablePtr::new(unsafe { self.value.ptr as *mut GcTable }))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_string_ptr(&self) -> Option<StringPtr> {
        if self.ttisstring() {
            Some(StringPtr::new(unsafe { self.value.ptr as *mut GcString }))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_binary_ptr(&self) -> Option<BinaryPtr> {
        if self.ttisbinary() {
            Some(BinaryPtr::new(unsafe { self.value.ptr as *mut GcBinary }))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_function_ptr(&self) -> Option<FunctionPtr> {
        if self.ttisfunction() {
            Some(FunctionPtr::new(unsafe {
                self.value.ptr as *mut GcFunction
            }))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_cclosure_ptr(&self) -> Option<CClosurePtr> {
        if self.is_cclosure() {
            Some(CClosurePtr::new(unsafe {
                self.value.ptr as *mut GcCClosure
            }))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_userdata_ptr(&self) -> Option<UserdataPtr> {
        if self.ttisfulluserdata() {
            Some(UserdataPtr::new(unsafe {
                self.value.ptr as *mut GcUserdata
            }))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_thread_ptr(&self) -> Option<ThreadPtr> {
        if self.ttisthread() {
            Some(ThreadPtr::new(unsafe { self.value.ptr as *mut GcThread }))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_gc_ptr(&self) -> Option<GcObjectPtr> {
        match self.kind() {
            LuaValueKind::Table => self.as_table_ptr().map(GcObjectPtr::Table),
            LuaValueKind::Function => self.as_function_ptr().map(GcObjectPtr::Function),
            LuaValueKind::CClosure => self.as_cclosure_ptr().map(GcObjectPtr::CClosure),
            LuaValueKind::String => self.as_string_ptr().map(GcObjectPtr::String),
            LuaValueKind::Binary => self.as_binary_ptr().map(GcObjectPtr::Binary),
            LuaValueKind::Thread => self.as_thread_ptr().map(GcObjectPtr::Thread),
            LuaValueKind::Userdata => self.as_userdata_ptr().map(GcObjectPtr::Userdata),
            _ => None,
        }
    }

    // ============ Truthiness (Lua semantics) ============

    /// l_isfalse - Lua truthiness: only nil and false are falsy
    #[inline(always)]
    pub fn is_truthy(&self) -> bool {
        !self.is_falsy()
    }

    #[inline(always)]
    pub fn is_falsy(&self) -> bool {
        self.ttisnil() || self.ttisfalse()
    }

    // ============ Type name ============

    pub fn type_name(&self) -> &'static str {
        match self.ttype() {
            LUA_TNIL => "nil",
            LUA_TBOOLEAN => "boolean",
            LUA_TNUMBER => "number",
            LUA_TSTRING => "string",
            LUA_TTABLE => "table",
            LUA_TFUNCTION => "function",
            LUA_TLIGHTUSERDATA => "userdata",
            LUA_TUSERDATA => "userdata",
            LUA_TTHREAD => "thread",
            _ => "unknown",
        }
    }

    // ============ Kind enum (for pattern matching) ============

    pub fn kind(&self) -> LuaValueKind {
        match self.ttype() {
            LUA_TNIL => LuaValueKind::Nil,
            LUA_TBOOLEAN => LuaValueKind::Boolean,
            LUA_TNUMBER => {
                if self.ttisinteger() {
                    LuaValueKind::Integer
                } else {
                    LuaValueKind::Float
                }
            }
            LUA_TSTRING => {
                if self.ttisbinary() {
                    LuaValueKind::Binary
                } else {
                    LuaValueKind::String
                }
            }
            LUA_TTABLE => LuaValueKind::Table,
            LUA_TFUNCTION => {
                if self.ttiscfunction() {
                    LuaValueKind::CFunction
                } else if self.is_cclosure() {
                    LuaValueKind::CClosure
                } else {
                    LuaValueKind::Function
                }
            }
            LUA_TLIGHTUSERDATA | LUA_TUSERDATA => LuaValueKind::Userdata,
            LUA_TTHREAD => LuaValueKind::Thread,
            _ => LuaValueKind::Nil,
        }
    }

    pub fn raw_ptr_repr(&self) -> *const u8 {
        unsafe { self.value.ptr }
    }

    /// Get hash value for this LuaValue (for native table implementation)
    #[inline(always)]
    pub fn hash_value(&self) -> u64 {
        let tt = self.tt();

        // Fast path for strings: use precomputed hash
        if tt == LUA_VSHRSTR || tt == LUA_VLNGSTR {
            return unsafe { (*(self.value.ptr as *const GcString)).data.hash };
        }

        // For integers, use direct value
        if tt == LUA_VNUMINT {
            return unsafe { self.value.i as u64 };
        }

        // For other types, use pointer or value bits
        unsafe { self.value.i as u64 }
    }

    /// Get hash value for a string value (avoids type check).
    /// SAFETY: caller must guarantee self is a string (short or long).
    #[inline(always)]
    pub unsafe fn hash_string_unchecked(&self) -> u64 {
        unsafe { (*(self.value.ptr as *const GcString)).data.hash }
    }
}

/// Check if a float value exactly equals an integer value.
/// Returns false if the float can't precisely represent the integer.
#[inline(always)]
fn lua_float_eq_int(f: f64, i: i64) -> bool {
    if !f.is_finite() {
        return false;
    }
    // Check if float is integral and round-trips through i64
    if f != f.floor() {
        return false;
    }
    // Check range (i64::MIN is exactly representable as f64, i64::MAX is not)
    if f < i64::MIN as f64 || f >= (i64::MAX as f64) + 1.0 {
        return false;
    }
    (f as i64) == i
}

impl PartialEq for LuaValue {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        let tt = self.tt();
        let other_tt = other.tt();

        // Fast path: same type tag and same value bits
        if tt == other_tt {
            // For all types except float:
            // - nil/boolean: value.i is 0/1, direct compare works
            // - integer: direct i64 compare
            // - string/table/function/etc: pointer compare (interned strings have same pointer)
            // Float must use f64 compare so that NaN != NaN (IEEE 754)
            if tt == LUA_VNUMFLT {
                return unsafe { self.value.n == other.value.n };
            }
            if unsafe { self.value.i == other.value.i } {
                return true;
            }
            return match tt {
                LUA_VSHRSTR => false, // different pointers, different interned strings
                LUA_VLNGSTR => {
                    let s1 = unsafe { &*(self.value.ptr as *const GcString) };
                    let s2 = unsafe { &*(other.value.ptr as *const GcString) };
                    s1.data.hash == s2.data.hash && s1.data.str == s2.data.str
                }
                LUA_VBINARY => {
                    let b1 = unsafe { &*(self.value.ptr as *const GcBinary) };
                    let b2 = unsafe { &*(other.value.ptr as *const GcBinary) };
                    b1.data == b2.data
                }
                _ => false,
            };
        } else if tt == LUA_VNUMINT && other_tt == LUA_VNUMFLT {
            let f = other.fltvalue();
            let i = self.ivalue();
            // Only equal if float exactly represents this integer
            return lua_float_eq_int(f, i);
        } else if tt == LUA_VNUMFLT && other_tt == LUA_VNUMINT {
            let f = self.fltvalue();
            let i = other.ivalue();
            return lua_float_eq_int(f, i);
        } else if (tt == LUA_VSHRSTR || tt == LUA_VLNGSTR) && other_tt == LUA_VBINARY {
            // Compare string with binary - compare bytes
            let str_bytes = if tt == LUA_VSHRSTR {
                unsafe { &*(self.value.ptr as *const GcString) }
                    .data
                    .str
                    .as_bytes()
            } else {
                unsafe { &*(self.value.ptr as *const GcString) }
                    .data
                    .str
                    .as_bytes()
            };
            let binary_bytes = &unsafe { &*(other.value.ptr as *const GcBinary) }.data;
            return str_bytes == binary_bytes.as_slice();
        } else if tt == LUA_VBINARY && (other_tt == LUA_VSHRSTR || other_tt == LUA_VLNGSTR) {
            // Compare binary with string - compare bytes
            let binary_bytes = &unsafe { &*(self.value.ptr as *const GcBinary) }.data;
            let str_bytes = if other_tt == LUA_VSHRSTR {
                unsafe { &*(other.value.ptr as *const GcString) }
                    .data
                    .str
                    .as_bytes()
            } else {
                unsafe { &*(other.value.ptr as *const GcString) }
                    .data
                    .str
                    .as_bytes()
            };
            return binary_bytes.as_slice() == str_bytes;
        }
        false
    }
}

// Lua tables can use floats as keys, so we implement Eq even though it's not strictly correct
// This is fine because NaN values are rare as table keys in Lua
impl Eq for LuaValue {}

// ============ Type enum for pattern matching ============

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LuaValueKind {
    Nil,
    Boolean,
    Integer,
    Float,
    String,
    Binary,
    Table,
    Function,
    CFunction,
    CClosure,
    Userdata,
    Thread,
}

// ============ Traits ============

impl Default for LuaValue {
    #[inline(always)]
    fn default() -> Self {
        Self::nil()
    }
}

impl std::fmt::Debug for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            LuaValueKind::Nil => write!(f, "nil"),
            LuaValueKind::Boolean => write!(f, "{}", self.bvalue()),
            LuaValueKind::Integer => write!(f, "{}", self.ivalue()),
            LuaValueKind::Float => write!(f, "{}", self.fltvalue()),
            LuaValueKind::String => {
                write!(f, "\"{}\"", self.as_str().unwrap_or("<invalid string>"))
            }
            LuaValueKind::Binary => write!(f, "binary(0x{:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::Table => write!(f, "table(0x{:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::Function => write!(f, "function(0x{:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::CFunction => {
                write!(f, "cfunction(0x{:#x})", self.raw_ptr_repr() as usize)
            }
            LuaValueKind::CClosure => {
                write!(f, "cclosure(0x{:#x})", self.raw_ptr_repr() as usize)
            }
            LuaValueKind::Userdata => write!(f, "userdata(0x{:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::Thread => write!(f, "thread(0x{:#x})", self.raw_ptr_repr() as usize),
        }
    }
}

impl std::fmt::Display for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            LuaValueKind::Nil => write!(f, "nil"),
            LuaValueKind::Boolean => write!(f, "{}", self.bvalue()),
            LuaValueKind::Integer => write!(f, "{}", self.ivalue()),
            LuaValueKind::Float => {
                let n = self.fltvalue();
                if n.floor() == n && n.abs() < 1e14 {
                    write!(f, "{:.0}", n)
                } else {
                    write!(f, "{}", n)
                }
            }
            LuaValueKind::String => write!(f, "{}", self.as_str().unwrap_or("<invalid string>")),
            LuaValueKind::Table => write!(f, "table: 0x{:x}", unsafe { self.value.ptr as usize }),
            LuaValueKind::Function => {
                write!(f, "function: 0x{:x}", unsafe { self.value.ptr as usize })
            }
            LuaValueKind::CFunction => write!(f, "function: 0x{:x}", unsafe { self.value.f }),
            LuaValueKind::CClosure => {
                write!(f, "function: 0x{:x}", unsafe { self.value.ptr as usize })
            }
            LuaValueKind::Userdata => {
                write!(f, "userdata: 0x{:x}", unsafe { self.value.ptr as usize })
            }
            LuaValueKind::Thread => {
                write!(f, "thread: 0x{:x}", unsafe { self.value.ptr as usize })
            }
            LuaValueKind::Binary => {
                write!(f, "binary: 0x{:x}", unsafe { self.value.ptr as usize })
            }
        }
    }
}

impl std::hash::Hash for LuaValue {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let tt = self.tt();

        // Fast path for strings: use precomputed hash directly
        // This is the most common case for table lookups
        // Both short strings (LUA_VSHRSTR = 0x44) and long strings (LUA_VLNGSTR = 0x54)
        if tt == LUA_VSHRSTR || tt == LUA_VLNGSTR {
            // For strings, just hash the precomputed hash value
            // No need to hash tt since string vs binary is distinguished in eq()
            let hash = unsafe { (*(self.value.ptr as *const GcString)).data.hash };
            hash.hash(state);
            return;
        }

        // Special handling for numbers to maintain equality invariant
        // (integer 1 == float 1.0, so they must hash the same)
        if tt == LUA_VNUMINT || tt == LUA_VNUMFLT {
            // Always hash numbers as floats to maintain hash consistency
            // when integer equals float
            unsafe {
                let n = if tt == LUA_VNUMINT {
                    self.value.i as f64
                } else {
                    self.value.n
                };
                // Use a stable representation for hashing
                LUA_TNUMBER.hash(state);
                n.to_bits().hash(state);
            }
        } else if tt <= LUA_VFALSE {
            // nil or boolean - hash type tag only
            tt.hash(state);
        } else {
            // Other GC types: hash type tag + pointer (they use identity for equality)
            tt.hash(state);
            self.raw_ptr_repr().hash(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size() {
        assert_eq!(std::mem::size_of::<LuaValue>(), 16);
        assert_eq!(std::mem::size_of::<Value>(), 8);
    }

    #[test]
    fn test_nil() {
        let v = LuaValue::nil();
        assert!(v.ttisnil());
        assert!(v.ttisstrictnil());
        assert!(v.is_falsy());
        assert_eq!(v.type_name(), "nil");
    }

    #[test]
    fn test_empty() {
        let v = LuaValue::empty();
        assert!(v.ttisnil()); // empty is a nil variant
        assert!(v.ttisempty());
        assert!(!v.ttisstrictnil());
    }

    #[test]
    fn test_boolean() {
        let t = LuaValue::boolean(true);
        let f = LuaValue::boolean(false);

        assert!(t.ttisboolean());
        assert!(f.ttisboolean());
        assert!(t.ttistrue());
        assert!(f.ttisfalse());
        assert_eq!(t.bvalue(), true);
        assert_eq!(f.bvalue(), false);
        assert!(t.is_truthy());
        assert!(f.is_falsy());
    }

    #[test]
    fn test_integer() {
        let v = LuaValue::integer(42);
        assert!(v.ttisnumber());
        assert!(v.ttisinteger());
        assert_eq!(v.ivalue(), 42);
        assert_eq!(v.nvalue(), 42.0);

        let neg = LuaValue::integer(-100);
        assert_eq!(neg.ivalue(), -100);
    }

    #[test]
    fn test_float() {
        let v = LuaValue::float(3.14);
        assert!(v.ttisnumber());
        assert!(v.ttisfloat());
        assert!((v.fltvalue() - 3.14).abs() < f64::EPSILON);
        assert!((v.nvalue() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_equality() {
        assert_eq!(LuaValue::nil(), LuaValue::nil());
        assert_eq!(LuaValue::integer(42), LuaValue::integer(42));
        assert_ne!(LuaValue::integer(42), LuaValue::integer(43));
    }

    #[test]
    fn test_type_tags() {
        assert_eq!(novariant(LUA_VNUMINT), LUA_TNUMBER);
        assert_eq!(novariant(LUA_VNUMFLT), LUA_TNUMBER);
        assert_eq!(withvariant(LUA_VNUMINT), LUA_VNUMINT);
        assert_eq!(makevariant!(LUA_TNUMBER, 0), LUA_VNUMINT);
        assert_eq!(makevariant!(LUA_TNUMBER, 1), LUA_VNUMFLT);
    }

    #[test]
    fn test_collectable_bit() {
        let nil = LuaValue::nil();
        let int = LuaValue::integer(42);
        assert!(!nil.iscollectable());
        assert!(!int.iscollectable());
    }
}
