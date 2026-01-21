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
use crate::lua_value::LuaUserdata;
use crate::lua_vm::{CFunction, LuaState};
use crate::{
    BinaryPtr, FunctionBody, FunctionPtr, GcBinary, GcFunction, GcObjectPtr, GcString, GcTable,
    GcThread, GcUserdata, LuaTable, StringPtr, TablePtr, ThreadPtr, UserdataPtr,
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
#[allow(unused)]
pub const LUA_VNOTABLE: u8 = makevariant!(LUA_TNIL, 3); // fast get non-table signal

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

pub const LUA_VSTR: u8 = LUA_TSTRING | BIT_ISCOLLECTABLE; // 0x44 - short string
pub const LUA_VBINARY: u8 = makevariant!(LUA_TSTRING, 1) | BIT_ISCOLLECTABLE; // 0x54 - binary data
pub const LUA_VTABLE: u8 = LUA_TTABLE | BIT_ISCOLLECTABLE; // 0x45
pub const LUA_VFUNCTION: u8 = LUA_TFUNCTION | BIT_ISCOLLECTABLE; // 0x46  
pub const LUA_VUSERDATA: u8 = LUA_TUSERDATA | BIT_ISCOLLECTABLE; // 0x47
pub const LUA_VTHREAD: u8 = LUA_TTHREAD | BIT_ISCOLLECTABLE; // 0x48

// Light C function (NOT collectable - function pointer stored directly)
pub const LUA_VLCF: u8 = 0x06; // makevariant(LUA_TFUNCTION, 0) - light C function

#[inline(always)]
pub const fn novariant(tt: u8) -> u8 {
    tt & 0x0F
}

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

    /// Create a string value from StringId
    #[inline(always)]
    pub fn string(ptr: StringPtr) -> Self {
        Self {
            value: Value {
                ptr: ptr.as_ptr() as *const u8,
            }, // Pointer set later
            tt: LUA_VSTR,
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

    /// rawtt(o) - raw type tag
    #[inline(always)]
    pub fn rawtt(&self) -> u8 {
        self.tt()
    }

    /// ttype(o) - type without variants (bits 0-3)
    #[inline(always)]
    pub fn ttype(&self) -> u8 {
        novariant(self.tt())
    }

    /// ttypetag(o) - type tag with variants (bits 0-5)
    #[inline(always)]
    pub fn ttypetag(&self) -> u8 {
        withvariant(self.tt())
    }

    /// checktag(o, t) - exact tag match
    #[inline(always)]
    pub fn checktag(&self, t: u8) -> bool {
        self.tt() == t
    }

    /// checktype(o, t) - type match (ignoring variants)
    #[inline(always)]
    pub fn checktype(&self, t: u8) -> bool {
        novariant(self.tt()) == t
    }

    /// iscollectable(o) - is this a GC object?
    #[inline(always)]
    pub fn iscollectable(&self) -> bool {
        (self.tt() & BIT_ISCOLLECTABLE) != 0
    }

    /// is_collectable - alias for Rust naming convention
    #[inline(always)]
    pub fn is_collectable(&self) -> bool {
        self.iscollectable()
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
    pub fn ttisbinary(&self) -> bool {
        self.checktag(LUA_VBINARY)
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
        self.tt() == LUA_VTRUE
    }

    #[inline(always)]
    pub fn ivalue(&self) -> i64 {
        debug_assert!(self.ttisinteger());
        unsafe { self.value.i }
    }

    #[inline(always)]
    pub fn fltvalue(&self) -> f64 {
        debug_assert!(self.ttisfloat());
        unsafe { self.value.n }
    }

    /// nvalue - convert any number to f64
    #[inline(always)]
    pub fn nvalue(&self) -> f64 {
        debug_assert!(self.ttisnumber());
        if self.ttisinteger() {
            unsafe { self.value.i as f64 }
        } else {
            unsafe { self.value.n }
        }
    }

    #[inline(always)]
    pub fn pvalue(&self) -> *mut std::ffi::c_void {
        debug_assert!(self.ttislightuserdata());
        unsafe { self.value.p }
    }

    #[inline(always)]
    pub fn fvalue(&self) -> CFunction {
        debug_assert!(self.ttiscfunction());
        unsafe { std::mem::transmute(self.value.f) }
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
    pub fn as_str(&self) -> Option<&str> {
        if self.ttisstring() {
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
    pub fn as_lua_function(&self) -> Option<&FunctionBody> {
        if self.ttisluafunction() {
            let func = unsafe { &*(self.value.ptr as *const GcFunction) };
            Some(&func.data)
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

    pub fn as_string_ptr(&self) -> Option<StringPtr> {
        if self.ttisstring() {
            Some(StringPtr::new(unsafe { self.value.ptr as *mut GcString }))
        } else {
            None
        }
    }

    pub fn as_binary_ptr(&self) -> Option<BinaryPtr> {
        if self.ttisbinary() {
            Some(BinaryPtr::new(unsafe { self.value.ptr as *mut GcBinary }))
        } else {
            None
        }
    }

    pub fn as_function_ptr(&self) -> Option<FunctionPtr> {
        if self.ttisfunction() {
            Some(FunctionPtr::new(unsafe {
                self.value.ptr as *mut GcFunction
            }))
        } else {
            None
        }
    }

    pub fn as_userdata_ptr(&self) -> Option<UserdataPtr> {
        if self.ttisfulluserdata() {
            Some(UserdataPtr::new(unsafe {
                self.value.ptr as *mut GcUserdata
            }))
        } else {
            None
        }
    }

    pub fn as_thread_ptr(&self) -> Option<ThreadPtr> {
        if self.ttisthread() {
            Some(ThreadPtr::new(unsafe { self.value.ptr as *mut GcThread }))
        } else {
            None
        }
    }

    pub fn as_gc_ptr(&self) -> Option<GcObjectPtr> {
        match self.kind() {
            LuaValueKind::Table => self.as_table_ptr().map(GcObjectPtr::Table),
            LuaValueKind::Function => self.as_function_ptr().map(GcObjectPtr::Function),
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
                } else {
                    LuaValueKind::Function
                }
            }
            LUA_TLIGHTUSERDATA | LUA_TUSERDATA => LuaValueKind::Userdata,
            LUA_TTHREAD => LuaValueKind::Thread,
            _ => LuaValueKind::Nil,
        }
    }

    fn raw_ptr_repr(&self) -> *const u8 {
        unsafe { self.value.ptr }
    }
}

impl PartialEq for LuaValue {
    fn eq(&self, other: &Self) -> bool {
        if self.tt() == other.tt() {
            if unsafe { self.value.i == other.value.i } {
                return true;
            } else if self.ttisstring() {
                // Compare string contents
                let s1 = unsafe { &*(self.value.ptr as *const GcString) };
                let s2 = unsafe { &*(other.value.ptr as *const GcString) };
                return &s1.data == &s2.data;
            }

            return false;
        } else if self.ttisinteger() && other.ttisfloat() {
            return self.ivalue() as f64 == other.fltvalue();
        } else if self.ttisfloat() && other.ttisinteger() {
            return self.fltvalue() == other.ivalue() as f64;
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
            LuaValueKind::Binary => write!(f, "binary({:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::Table => write!(f, "table({:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::Function => write!(f, "function({:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::CFunction => write!(f, "cfunction({:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::Userdata => write!(f, "userdata({:#x})", self.raw_ptr_repr() as usize),
            LuaValueKind::Thread => write!(f, "thread({:#x})", self.raw_ptr_repr() as usize),
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
            LuaValueKind::Table => write!(f, "table({:x})", unsafe { self.value.ptr as usize }),
            LuaValueKind::Function => {
                write!(f, "function({:x})", unsafe { self.value.ptr as usize })
            }
            LuaValueKind::CFunction => write!(f, "cfunction({:x})", unsafe { self.value.f }),
            LuaValueKind::Userdata => {
                write!(f, "userdata({:x})", unsafe { self.value.ptr as usize })
            }
            LuaValueKind::Thread => write!(f, "thread({:x})", unsafe { self.value.ptr as usize }),
            LuaValueKind::Binary => {
                write!(f, "binary({:x})", unsafe { self.value.ptr as usize })
            }
        }
    }
}

impl std::hash::Hash for LuaValue {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let tt = self.tt();

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
            // GC types: hash type tag + gc_id
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
