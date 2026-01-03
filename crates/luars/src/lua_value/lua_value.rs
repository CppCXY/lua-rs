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

use crate::ObjectPool;
use crate::gc::{FunctionId, StringId, TableId, ThreadId, UserdataId};
use crate::lua_vm::CFunction;

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

// Extra types for non-values
pub const LUA_NUMTYPES: u8 = 9;
pub const LUA_TUPVAL: u8 = LUA_NUMTYPES; // upvalues
pub const LUA_TPROTO: u8 = LUA_NUMTYPES + 1; // function prototypes  
pub const LUA_TDEADKEY: u8 = LUA_NUMTYPES + 2; // removed keys in tables

// ============ Variant tags (with bits 4-5) ============
// makevariant(t,v) = ((t) | ((v) << 4))

// Nil variants
pub const LUA_VNIL: u8 = 0; // makevariant(LUA_TNIL, 0)
pub const LUA_VEMPTY: u8 = 0x10; // makevariant(LUA_TNIL, 1) - empty slot in table
pub const LUA_VABSTKEY: u8 = 0x20; // makevariant(LUA_TNIL, 2) - absent key in table
pub const LUA_VNOTABLE: u8 = 0x30; // makevariant(LUA_TNIL, 3) - fast get non-table signal

// Boolean variants
pub const LUA_VFALSE: u8 = 0x01; // makevariant(LUA_TBOOLEAN, 0)
pub const LUA_VTRUE: u8 = 0x11; // makevariant(LUA_TBOOLEAN, 1)

// Number variants
pub const LUA_VNUMINT: u8 = 0x03; // makevariant(LUA_TNUMBER, 0) - integer
pub const LUA_VNUMFLT: u8 = 0x13; // makevariant(LUA_TNUMBER, 1) - float

// String variants (bit 6 set = collectable)
pub const LUA_VSHRSTR: u8 = 0x44; // makevariant(LUA_TSTRING, 0) | BIT_ISCOLLECTABLE
pub const LUA_VLNGSTR: u8 = 0x54; // makevariant(LUA_TSTRING, 1) | BIT_ISCOLLECTABLE

// Light userdata (NOT collectable)
pub const LUA_VLIGHTUSERDATA: u8 = 0x02; // makevariant(LUA_TLIGHTUSERDATA, 0)

// Collectable types (bit 6 set)
pub const BIT_ISCOLLECTABLE: u8 = 1 << 6;

pub const LUA_VTABLE: u8 = LUA_TTABLE | BIT_ISCOLLECTABLE; // 0x45
pub const LUA_VFUNCTION: u8 = LUA_TFUNCTION | BIT_ISCOLLECTABLE; // 0x46  
pub const LUA_VUSERDATA: u8 = LUA_TUSERDATA | BIT_ISCOLLECTABLE; // 0x47
pub const LUA_VTHREAD: u8 = LUA_TTHREAD | BIT_ISCOLLECTABLE; // 0x48

// Light C function (NOT collectable - function pointer stored directly)
pub const LUA_VLCF: u8 = 0x06; // makevariant(LUA_TFUNCTION, 0) - light C function

// Helper macros as functions
#[inline(always)]
pub const fn makevariant(t: u8, v: u8) -> u8 {
    t | (v << 4)
}

#[inline(always)]
pub const fn novariant(tt: u8) -> u8 {
    tt & 0x0F
}

#[inline(always)]
pub const fn withvariant(tt: u8) -> u8 {
    tt & 0x3F
}

/// ctb - mark a tag as collectable
#[inline(always)]
pub const fn ctb(t: u8) -> u8 {
    t | BIT_ISCOLLECTABLE
}

/// tagisempty - check if tag represents empty slot
#[inline(always)]
pub const fn tagisempty(tag: u8) -> bool {
    novariant(tag) == LUA_TNIL
}

// ============ Value union ============
/// Lua 5.5 Value union (8 bytes)
/// Corresponds to union Value in lobject.h
#[derive(Clone, Copy)]
#[repr(C)]
pub union Value {
    pub gc_id: u32, // GC object ID (String/Table/Function/Userdata/Thread)
    pub p: u64,     // light userdata pointer
    pub f: u64,     // light C function pointer
    pub i: i64,     // integer number
    pub n: f64,     // float number
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
    pub fn gc(id: u32) -> Self {
        Value { gc_id: id }
    }

    #[inline(always)]
    pub fn lightuserdata(p: *mut std::ffi::c_void) -> Self {
        Value { p: p as u64 }
    }

    #[inline(always)]
    pub fn cfunction(f: CFunction) -> Self {
        Value {
            f: f as usize as u64,
        }
    }
}

// ============ TValue ============
/// Lua 5.5 TValue structure (9 bytes + 7 bytes padding = 16 bytes)
/// Corresponds to struct TValue in lobject.h
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuaValue {
    pub value_: Value, // 8 bytes
    pub tt_: u8,       // 1 byte type tag
                       // 7 bytes padding for 16-byte alignment
}

impl LuaValue {
    // ============ Constructors ============

    #[inline(always)]
    pub const fn nil() -> Self {
        Self {
            value_: Value::nil(),
            tt_: LUA_VNIL,
        }
    }

    #[inline(always)]
    pub const fn empty() -> Self {
        Self {
            value_: Value::nil(),
            tt_: LUA_VEMPTY,
        }
    }

    #[inline(always)]
    pub const fn abstkey() -> Self {
        Self {
            value_: Value::nil(),
            tt_: LUA_VABSTKEY,
        }
    }

    #[inline(always)]
    pub const fn boolean(b: bool) -> Self {
        Self {
            value_: Value::nil(),
            tt_: if b { LUA_VTRUE } else { LUA_VFALSE },
        }
    }

    #[inline(always)]
    pub const fn integer(i: i64) -> Self {
        Self {
            value_: Value::integer(i),
            tt_: LUA_VNUMINT,
        }
    }

    #[inline(always)]
    pub fn float(n: f64) -> Self {
        Self {
            value_: Value::float(n),
            tt_: LUA_VNUMFLT,
        }
    }

    #[inline(always)]
    pub fn number(n: f64) -> Self {
        Self::float(n)
    }

    // String constructors
    #[inline(always)]
    pub fn shrstring(id: StringId) -> Self {
        Self {
            value_: Value::gc(id.index()),
            tt_: LUA_VSHRSTR,
        }
    }

    #[inline(always)]
    pub fn lngstring(id: StringId) -> Self {
        Self {
            value_: Value::gc(id.index()),
            tt_: LUA_VLNGSTR,
        }
    }

    /// Create a string value from StringId (automatically selects short/long based on flag)
    #[inline(always)]
    pub fn string(id: StringId) -> Self {
        if id.is_short() {
            Self::shrstring(id)
        } else {
            Self::lngstring(id)
        }
    }

    #[inline(always)]
    pub fn table(id: TableId) -> Self {
        Self {
            value_: Value::gc(id.0),
            tt_: LUA_VTABLE,
        }
    }

    #[inline(always)]
    pub fn function(id: FunctionId) -> Self {
        Self {
            value_: Value::gc(id.0),
            tt_: LUA_VFUNCTION,
        }
    }

    #[inline(always)]
    pub fn function_id(id: FunctionId) -> Self {
        Self::function(id)
    }

    // Light C function (NOT collectable)
    #[inline(always)]
    pub fn cfunction(f: CFunction) -> Self {
        Self {
            value_: Value::cfunction(f),
            tt_: LUA_VLCF,
        }
    }

    #[inline(always)]
    pub fn cfunction_ptr(f_ptr: usize) -> Self {
        Self {
            value_: Value { f: f_ptr as u64 },
            tt_: LUA_VLCF,
        }
    }

    #[inline(always)]
    pub fn lightuserdata(p: *mut std::ffi::c_void) -> Self {
        Self {
            value_: Value::lightuserdata(p),
            tt_: LUA_VLIGHTUSERDATA,
        }
    }

    #[inline(always)]
    pub fn userdata(id: UserdataId) -> Self {
        Self {
            value_: Value::gc(id.0),
            tt_: LUA_VUSERDATA,
        }
    }

    #[inline(always)]
    pub fn thread(id: ThreadId) -> Self {
        Self {
            value_: Value::gc(id.0),
            tt_: LUA_VTHREAD,
        }
    }

    // ============ Type checking (following Lua 5.5 macros) ============

    /// rawtt(o) - raw type tag
    #[inline(always)]
    pub fn rawtt(&self) -> u8 {
        self.tt_
    }

    /// ttype(o) - type without variants (bits 0-3)
    #[inline(always)]
    pub fn ttype(&self) -> u8 {
        novariant(self.tt_)
    }

    /// ttypetag(o) - type tag with variants (bits 0-5)
    #[inline(always)]
    pub fn ttypetag(&self) -> u8 {
        withvariant(self.tt_)
    }

    /// checktag(o, t) - exact tag match
    #[inline(always)]
    pub fn checktag(&self, t: u8) -> bool {
        self.tt_ == t
    }

    /// checktype(o, t) - type match (ignoring variants)
    #[inline(always)]
    pub fn checktype(&self, t: u8) -> bool {
        novariant(self.tt_) == t
    }

    /// iscollectable(o) - is this a GC object?
    #[inline(always)]
    pub fn iscollectable(&self) -> bool {
        (self.tt_ & BIT_ISCOLLECTABLE) != 0
    }

    // Specific type checks
    #[inline(always)]
    pub fn ttisnil(&self) -> bool {
        self.checktype(LUA_TNIL)
    }

    #[inline(always)]
    pub fn ttisstrictnil(&self) -> bool {
        self.checktag(LUA_VNIL)
    }

    #[inline(always)]
    pub fn ttisempty(&self) -> bool {
        self.checktag(LUA_VEMPTY)
    }

    #[inline(always)]
    pub fn isabstkey(&self) -> bool {
        self.checktag(LUA_VABSTKEY)
    }

    #[inline(always)]
    pub fn ttisboolean(&self) -> bool {
        self.checktype(LUA_TBOOLEAN)
    }

    #[inline(always)]
    pub fn ttisfalse(&self) -> bool {
        self.checktag(LUA_VFALSE)
    }

    #[inline(always)]
    pub fn ttistrue(&self) -> bool {
        self.checktag(LUA_VTRUE)
    }

    #[inline(always)]
    pub fn ttisnumber(&self) -> bool {
        self.checktype(LUA_TNUMBER)
    }

    #[inline(always)]
    pub fn ttisinteger(&self) -> bool {
        self.checktag(LUA_VNUMINT)
    }

    #[inline(always)]
    pub fn ttisfloat(&self) -> bool {
        self.checktag(LUA_VNUMFLT)
    }

    #[inline(always)]
    pub fn ttisstring(&self) -> bool {
        self.checktype(LUA_TSTRING)
    }

    #[inline(always)]
    pub fn ttisshrstring(&self) -> bool {
        self.checktag(LUA_VSHRSTR)
    }

    #[inline(always)]
    pub fn ttislngstring(&self) -> bool {
        self.checktag(LUA_VLNGSTR)
    }

    #[inline(always)]
    pub fn ttistable(&self) -> bool {
        self.checktag(LUA_VTABLE)
    }

    #[inline(always)]
    pub fn ttisfunction(&self) -> bool {
        self.checktype(LUA_TFUNCTION)
    }

    #[inline(always)]
    pub fn ttisluafunction(&self) -> bool {
        self.checktag(LUA_VFUNCTION)
    }

    #[inline(always)]
    pub fn ttiscfunction(&self) -> bool {
        self.checktag(LUA_VLCF)
    }

    #[inline(always)]
    pub fn ttislightuserdata(&self) -> bool {
        self.checktag(LUA_VLIGHTUSERDATA)
    }

    #[inline(always)]
    pub fn ttisfulluserdata(&self) -> bool {
        self.checktag(LUA_VUSERDATA)
    }

    #[inline(always)]
    pub fn ttisthread(&self) -> bool {
        self.checktag(LUA_VTHREAD)
    }

    // ============ Value extraction ============

    #[inline(always)]
    pub fn bvalue(&self) -> bool {
        debug_assert!(self.ttisboolean());
        self.tt_ == LUA_VTRUE
    }

    #[inline(always)]
    pub fn ivalue(&self) -> i64 {
        debug_assert!(self.ttisinteger());
        unsafe { self.value_.i }
    }

    #[inline(always)]
    pub fn fltvalue(&self) -> f64 {
        debug_assert!(self.ttisfloat());
        unsafe { self.value_.n }
    }

    /// nvalue - convert any number to f64
    #[inline(always)]
    pub fn nvalue(&self) -> f64 {
        debug_assert!(self.ttisnumber());
        if self.ttisinteger() {
            unsafe { self.value_.i as f64 }
        } else {
            unsafe { self.value_.n }
        }
    }

    #[inline(always)]
    pub fn pvalue(&self) -> *mut std::ffi::c_void {
        debug_assert!(self.ttislightuserdata());
        unsafe { self.value_.p as *mut std::ffi::c_void }
    }

    #[inline(always)]
    pub fn gcvalue(&self) -> u32 {
        debug_assert!(self.iscollectable());
        unsafe { self.value_.gc_id }
    }

    // Specific GC type extraction
    #[inline(always)]
    pub fn tsvalue(&self) -> StringId {
        debug_assert!(self.ttisstring());
        let index = unsafe { self.value_.gc_id };
        // Reconstruct StringId with correct long/short flag based on type tag
        if self.ttisshrstring() {
            StringId::short(index)
        } else {
            StringId::long(index)
        }
    }

    #[inline(always)]
    pub fn hvalue(&self) -> TableId {
        debug_assert!(self.ttistable());
        TableId(unsafe { self.value_.gc_id })
    }

    #[inline(always)]
    pub fn clvalue(&self) -> FunctionId {
        debug_assert!(self.ttisluafunction());
        FunctionId(unsafe { self.value_.gc_id })
    }

    #[inline(always)]
    pub fn fvalue(&self) -> CFunction {
        debug_assert!(self.ttiscfunction());
        unsafe { std::mem::transmute(self.value_.f as usize) }
    }

    #[inline(always)]
    pub fn uvalue(&self) -> UserdataId {
        debug_assert!(self.ttisfulluserdata());
        UserdataId(unsafe { self.value_.gc_id })
    }

    #[inline(always)]
    pub fn thvalue(&self) -> ThreadId {
        debug_assert!(self.ttisthread());
        ThreadId(unsafe { self.value_.gc_id })
    }

    // ============ Compatibility layer with old API ============

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
            // Lua 5.4 semantics: floats with zero fraction are integers
            let f = self.fltvalue();
            if f.fract() == 0.0 && f.is_finite() {
                const MIN_INT_F: f64 = i64::MIN as f64;
                const MAX_INT_F: f64 = i64::MAX as f64;
                if f >= MIN_INT_F && f <= MAX_INT_F {
                    Some(f as i64)
                } else {
                    None
                }
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
    pub fn as_string_id(&self) -> Option<StringId> {
        if self.ttisstring() {
            Some(self.tsvalue())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_table_id(&self) -> Option<TableId> {
        if self.ttistable() {
            Some(self.hvalue())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_function_id(&self) -> Option<FunctionId> {
        if self.ttisluafunction() {
            Some(self.clvalue())
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
    pub fn as_userdata_id(&self) -> Option<UserdataId> {
        if self.ttisfulluserdata() {
            Some(self.uvalue())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_thread_id(&self) -> Option<ThreadId> {
        if self.ttisthread() {
            Some(self.thvalue())
        } else {
            None
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

    // ============ Equality ============

    pub fn raw_equal(&self, other: &LuaValue, pool: &ObjectPool) -> bool {
        // Fast path: if type tags differ, not equal
        if self.tt_ != other.tt_ {
            return false;
        }

        // Type-specific comparison
        match self.ttype() {
            LUA_TNIL => true,                      // All nils are equal
            LUA_TBOOLEAN => self.tt_ == other.tt_, // Already checked above
            LUA_TNUMBER => {
                // Both are numbers, but could be integer or float
                if self.ttisinteger() && other.ttisinteger() {
                    self.ivalue() == other.ivalue()
                } else {
                    // Convert both to float for comparison
                    let v1 = if self.ttisinteger() { self.ivalue() as f64 } else { self.fltvalue() };
                    let v2 = if other.ttisinteger() { other.ivalue() as f64 } else { other.fltvalue() };
                    v1 == v2
                }
            }
            LUA_TSTRING => {
                let sid1 = self.tsvalue();
                let sid2 = other.tsvalue();

                // Short strings are interned, so ID comparison is enough
                if self.ttisshrstring() && other.ttisshrstring() {
                    return sid1.index() == sid2.index();
                }

                // Long strings need content comparison
                let s1 = pool.get_string(sid1);
                let s2 = pool.get_string(sid2);
                s1 == s2
            }
            LUA_TLIGHTUSERDATA => unsafe { self.value_.p == other.value_.p },
            LUA_TFUNCTION => {
                if self.ttiscfunction() {
                    unsafe { self.value_.f == other.value_.f }
                } else {
                    unsafe { self.value_.gc_id == other.value_.gc_id }
                }
            }
            // All other collectable types: compare GC IDs
            _ if self.iscollectable() => unsafe { self.value_.gc_id == other.value_.gc_id },
            _ => false,
        }
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
            LUA_TSTRING => LuaValueKind::String,
            LUA_TTABLE => LuaValueKind::Table,
            LUA_TFUNCTION => {
                if self.ttiscfunction() {
                    LuaValueKind::CFunction
                } else {
                    LuaValueKind::Function
                }
            }
            LUA_TLIGHTUSERDATA | LUA_TUSERDATA => LuaValueKind::Userdata,
            LUA_TTHREAD => LuaValueKind::Thread,
            _ => LuaValueKind::Nil,
        }
    }

    /// GC object ID for GC traversal
    #[inline(always)]
    pub fn gc_object_id(&self) -> Option<(u8, u32)> {
        if self.iscollectable() {
            Some((self.ttype(), unsafe { self.value_.gc_id }))
        } else {
            None
        }
    }
}

// ============ Type enum for pattern matching ============

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LuaValueKind {
    Nil,
    Boolean,
    Integer,
    Float,
    String,
    Table,
    Function,
    CFunction,
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
            LuaValueKind::String => write!(f, "string({})", self.tsvalue().raw()),
            LuaValueKind::Table => write!(f, "table({})", self.hvalue().0),
            LuaValueKind::Function => write!(f, "function({})", self.clvalue().0),
            LuaValueKind::CFunction => write!(f, "cfunction({:#x})", unsafe { self.value_.f }),
            LuaValueKind::Userdata => write!(f, "userdata({})", self.uvalue().0),
            LuaValueKind::Thread => write!(f, "thread({})", self.thvalue().0),
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
            LuaValueKind::String => write!(f, "string({})", self.tsvalue().raw()),
            LuaValueKind::Table => write!(f, "table: {:x}", self.hvalue().0),
            LuaValueKind::Function => write!(f, "function: {:x}", self.clvalue().0),
            LuaValueKind::CFunction => write!(f, "function: {:x}", unsafe { self.value_.f }),
            LuaValueKind::Userdata => write!(f, "userdata: {:x}", self.uvalue().0),
            LuaValueKind::Thread => write!(f, "thread: {:x}", self.thvalue().0),
        }
    }
}

impl std::hash::Hash for LuaValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.tt_.hash(state);
        // Hash the value based on type
        match self.ttype() {
            LUA_TNIL => {}
            LUA_TBOOLEAN => {}
            LUA_TNUMBER => unsafe {
                if self.ttisinteger() {
                    self.value_.i.hash(state);
                } else {
                    self.value_.n.to_bits().hash(state);
                }
            },
            _ => unsafe {
                // For all other types, hash the raw u64
                self.value_.i.hash(state);
            },
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
    fn test_string() {
        let v = LuaValue::string(StringId::short(123));
        assert!(v.ttisstring());
        assert!(v.ttisshrstring());
        assert_eq!(v.tsvalue(), StringId::short(123));

        let lng = LuaValue::lngstring(StringId::long(456));
        assert!(lng.ttisstring());
        assert!(lng.ttislngstring());
        assert_eq!(lng.tsvalue(), StringId::long(456));
    }

    #[test]
    fn test_table() {
        let v = LuaValue::table(TableId(789));
        assert!(v.ttistable());
        assert!(v.iscollectable());
        assert_eq!(v.hvalue(), TableId(789));
    }

    // #[test]
    // fn test_equality() {
    //     assert_eq!(LuaValue::nil(), LuaValue::nil());
    //     assert_eq!(LuaValue::integer(42), LuaValue::integer(42));
    //     assert_ne!(LuaValue::integer(42), LuaValue::integer(43));
    //     assert_eq!(LuaValue::table(TableId(1)), LuaValue::table(TableId(1)));
    //     assert_ne!(LuaValue::table(TableId(1)), LuaValue::table(TableId(2)));
    // }

    #[test]
    fn test_type_tags() {
        assert_eq!(novariant(LUA_VNUMINT), LUA_TNUMBER);
        assert_eq!(novariant(LUA_VNUMFLT), LUA_TNUMBER);
        assert_eq!(withvariant(LUA_VNUMINT), LUA_VNUMINT);
        assert_eq!(makevariant(LUA_TNUMBER, 0), LUA_VNUMINT);
        assert_eq!(makevariant(LUA_TNUMBER, 1), LUA_VNUMFLT);
    }

    #[test]
    fn test_collectable_bit() {
        let nil = LuaValue::nil();
        let int = LuaValue::integer(42);
        let str = LuaValue::string(StringId::short(1));
        let tbl = LuaValue::table(TableId(1));

        assert!(!nil.iscollectable());
        assert!(!int.iscollectable());
        assert!(str.iscollectable());
        assert!(tbl.iscollectable());
    }
}
