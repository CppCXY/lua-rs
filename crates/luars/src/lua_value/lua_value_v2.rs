// LuaValue V2 - Simplified design without pointer caching
// 
// Design principles:
// 1. No pointer caching - Arena/Vec may relocate data
// 2. All GC objects accessed via ID + ObjectPool lookup
// 3. Inline values (nil, bool, int, float, cfunc) stored directly
// 4. 16 bytes total for full i64/f64 support
//
// Layout:
// ┌──────────────────────────────────────────────────┐
// │ tag: u64                                          │
// │   High 16 bits: type tag                         │
// │   Low 32 bits: object ID (for GC objects)        │
// │                                                   │
// │ data: u64                                         │
// │   - Integer: i64 value                           │
// │   - Float: f64 bits                              │
// │   - CFunction: function pointer                  │
// │   - GC objects: unused (ID is in tag)            │
// └──────────────────────────────────────────────────┘

use crate::lua_value::CFunction;
use crate::object_pool_v2::{StringId, TableId, FunctionId, UpvalueId, UserdataId};

// Type tags (high 16 bits of tag field)
pub const TAG_NIL: u64       = 0x0000_0000_0000_0000;
pub const TAG_FALSE: u64     = 0x0001_0000_0000_0000;
pub const TAG_TRUE: u64      = 0x0002_0000_0000_0000;
pub const TAG_INTEGER: u64   = 0x0003_0000_0000_0000;
pub const TAG_FLOAT: u64     = 0x0004_0000_0000_0000;
pub const TAG_STRING: u64    = 0x0005_0000_0000_0000;
pub const TAG_TABLE: u64     = 0x0006_0000_0000_0000;
pub const TAG_FUNCTION: u64  = 0x0007_0000_0000_0000;
pub const TAG_CFUNCTION: u64 = 0x0008_0000_0000_0000;
pub const TAG_USERDATA: u64  = 0x0009_0000_0000_0000;
pub const TAG_UPVALUE: u64   = 0x000A_0000_0000_0000;
pub const TAG_THREAD: u64    = 0x000B_0000_0000_0000;

pub const TAG_MASK: u64 = 0xFFFF_0000_0000_0000;
pub const ID_MASK: u64  = 0x0000_0000_FFFF_FFFF;

/// Simplified LuaValue - no pointer caching
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuaValueV2 {
    tag: u64,   // type tag (high 16 bits) + object ID (low 32 bits)
    data: u64,  // i64/f64/cfunc pointer (for inline types)
}

// ============ Type enum for pattern matching ============

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuaTypeV2 {
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

impl LuaValueV2 {
    // ============ Constructors ============

    #[inline(always)]
    pub const fn nil() -> Self {
        Self { tag: TAG_NIL, data: 0 }
    }

    #[inline(always)]
    pub const fn boolean(b: bool) -> Self {
        Self {
            tag: if b { TAG_TRUE } else { TAG_FALSE },
            data: 0,
        }
    }

    #[inline(always)]
    pub const fn integer(i: i64) -> Self {
        Self {
            tag: TAG_INTEGER,
            data: i as u64,
        }
    }

    #[inline(always)]
    pub fn float(f: f64) -> Self {
        Self {
            tag: TAG_FLOAT,
            data: f.to_bits(),
        }
    }

    #[inline(always)]
    pub fn string(id: StringId) -> Self {
        Self {
            tag: TAG_STRING | (id.0 as u64),
            data: 0,
        }
    }

    #[inline(always)]
    pub fn table(id: TableId) -> Self {
        Self {
            tag: TAG_TABLE | (id.0 as u64),
            data: 0,
        }
    }

    #[inline(always)]
    pub fn function(id: FunctionId) -> Self {
        Self {
            tag: TAG_FUNCTION | (id.0 as u64),
            data: 0,
        }
    }

    #[inline(always)]
    pub fn cfunction(f: CFunction) -> Self {
        Self {
            tag: TAG_CFUNCTION,
            data: f as usize as u64,
        }
    }

    #[inline(always)]
    pub fn userdata(id: UserdataId) -> Self {
        Self {
            tag: TAG_USERDATA | (id.0 as u64),
            data: 0,
        }
    }

    #[inline(always)]
    pub fn upvalue(id: UpvalueId) -> Self {
        Self {
            tag: TAG_UPVALUE | (id.0 as u64),
            data: 0,
        }
    }

    // ============ Type checking ============

    #[inline(always)]
    pub fn get_type(&self) -> LuaTypeV2 {
        match self.tag & TAG_MASK {
            TAG_NIL => LuaTypeV2::Nil,
            TAG_FALSE | TAG_TRUE => LuaTypeV2::Boolean,
            TAG_INTEGER => LuaTypeV2::Integer,
            TAG_FLOAT => LuaTypeV2::Float,
            TAG_STRING => LuaTypeV2::String,
            TAG_TABLE => LuaTypeV2::Table,
            TAG_FUNCTION => LuaTypeV2::Function,
            TAG_CFUNCTION => LuaTypeV2::CFunction,
            TAG_USERDATA => LuaTypeV2::Userdata,
            TAG_THREAD => LuaTypeV2::Thread,
            _ => LuaTypeV2::Nil,
        }
    }

    #[inline(always)]
    pub fn is_nil(&self) -> bool {
        self.tag == TAG_NIL
    }

    #[inline(always)]
    pub fn is_boolean(&self) -> bool {
        matches!(self.tag & TAG_MASK, TAG_FALSE | TAG_TRUE)
    }

    #[inline(always)]
    pub fn is_integer(&self) -> bool {
        (self.tag & TAG_MASK) == TAG_INTEGER
    }

    #[inline(always)]
    pub fn is_float(&self) -> bool {
        (self.tag & TAG_MASK) == TAG_FLOAT
    }

    #[inline(always)]
    pub fn is_number(&self) -> bool {
        matches!(self.tag & TAG_MASK, TAG_INTEGER | TAG_FLOAT)
    }

    #[inline(always)]
    pub fn is_string(&self) -> bool {
        (self.tag & TAG_MASK) == TAG_STRING
    }

    #[inline(always)]
    pub fn is_table(&self) -> bool {
        (self.tag & TAG_MASK) == TAG_TABLE
    }

    #[inline(always)]
    pub fn is_function(&self) -> bool {
        matches!(self.tag & TAG_MASK, TAG_FUNCTION | TAG_CFUNCTION)
    }

    #[inline(always)]
    pub fn is_lua_function(&self) -> bool {
        (self.tag & TAG_MASK) == TAG_FUNCTION
    }

    #[inline(always)]
    pub fn is_cfunction(&self) -> bool {
        (self.tag & TAG_MASK) == TAG_CFUNCTION
    }

    // ============ Value extraction ============

    #[inline(always)]
    pub fn as_boolean(&self) -> Option<bool> {
        match self.tag {
            TAG_TRUE => Some(true),
            TAG_FALSE => Some(false),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_integer(&self) -> Option<i64> {
        if (self.tag & TAG_MASK) == TAG_INTEGER {
            Some(self.data as i64)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_float(&self) -> Option<f64> {
        if (self.tag & TAG_MASK) == TAG_FLOAT {
            Some(f64::from_bits(self.data))
        } else {
            None
        }
    }

    /// Get as number (integer or float)
    #[inline(always)]
    pub fn as_number(&self) -> Option<f64> {
        match self.tag & TAG_MASK {
            TAG_INTEGER => Some(self.data as i64 as f64),
            TAG_FLOAT => Some(f64::from_bits(self.data)),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_string_id(&self) -> Option<StringId> {
        if (self.tag & TAG_MASK) == TAG_STRING {
            Some(StringId((self.tag & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_table_id(&self) -> Option<TableId> {
        if (self.tag & TAG_MASK) == TAG_TABLE {
            Some(TableId((self.tag & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_function_id(&self) -> Option<FunctionId> {
        if (self.tag & TAG_MASK) == TAG_FUNCTION {
            Some(FunctionId((self.tag & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_cfunction(&self) -> Option<CFunction> {
        if (self.tag & TAG_MASK) == TAG_CFUNCTION {
            // SAFETY: We stored a valid function pointer
            Some(unsafe { std::mem::transmute(self.data as usize) })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_userdata_id(&self) -> Option<UserdataId> {
        if (self.tag & TAG_MASK) == TAG_USERDATA {
            Some(UserdataId((self.tag & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_upvalue_id(&self) -> Option<UpvalueId> {
        if (self.tag & TAG_MASK) == TAG_UPVALUE {
            Some(UpvalueId((self.tag & ID_MASK) as u32))
        } else {
            None
        }
    }

    // ============ Truthiness ============

    /// Lua truthiness: only nil and false are falsy
    #[inline(always)]
    pub fn is_truthy(&self) -> bool {
        !matches!(self.tag, TAG_NIL | TAG_FALSE)
    }

    #[inline(always)]
    pub fn is_falsy(&self) -> bool {
        matches!(self.tag, TAG_NIL | TAG_FALSE)
    }

    // ============ Object ID extraction (for GC) ============

    /// Get object ID if this is a GC object
    #[inline(always)]
    pub fn gc_object_id(&self) -> Option<(u8, u32)> {
        let type_tag = ((self.tag & TAG_MASK) >> 48) as u8;
        match type_tag {
            5..=11 => Some((type_tag, (self.tag & ID_MASK) as u32)),
            _ => None,
        }
    }

    // ============ Equality ============

    /// Raw equality (no metamethods)
    #[inline(always)]
    pub fn raw_equal(&self, other: &Self) -> bool {
        // For most types, both tag and data must match
        // Special case: NaN != NaN for floats
        if (self.tag & TAG_MASK) == TAG_FLOAT && (other.tag & TAG_MASK) == TAG_FLOAT {
            let a = f64::from_bits(self.data);
            let b = f64::from_bits(other.data);
            a == b  // This handles NaN correctly
        } else {
            self.tag == other.tag && self.data == other.data
        }
    }
}

impl Default for LuaValueV2 {
    fn default() -> Self {
        Self::nil()
    }
}

impl std::fmt::Debug for LuaValueV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.get_type() {
            LuaTypeV2::Nil => write!(f, "nil"),
            LuaTypeV2::Boolean => write!(f, "{}", self.tag == TAG_TRUE),
            LuaTypeV2::Integer => write!(f, "{}", self.data as i64),
            LuaTypeV2::Float => write!(f, "{}", f64::from_bits(self.data)),
            LuaTypeV2::String => write!(f, "string({})", (self.tag & ID_MASK)),
            LuaTypeV2::Table => write!(f, "table({})", (self.tag & ID_MASK)),
            LuaTypeV2::Function => write!(f, "function({})", (self.tag & ID_MASK)),
            LuaTypeV2::CFunction => write!(f, "cfunction({:#x})", self.data),
            LuaTypeV2::Userdata => write!(f, "userdata({})", (self.tag & ID_MASK)),
            LuaTypeV2::Thread => write!(f, "thread({})", (self.tag & ID_MASK)),
        }
    }
}

impl PartialEq for LuaValueV2 {
    fn eq(&self, other: &Self) -> bool {
        self.raw_equal(other)
    }
}

impl Eq for LuaValueV2 {}

impl std::hash::Hash for LuaValueV2 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash both tag and data
        self.tag.hash(state);
        self.data.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size() {
        assert_eq!(std::mem::size_of::<LuaValueV2>(), 16);
    }

    #[test]
    fn test_nil() {
        let v = LuaValueV2::nil();
        assert!(v.is_nil());
        assert!(v.is_falsy());
    }

    #[test]
    fn test_boolean() {
        let t = LuaValueV2::boolean(true);
        let f = LuaValueV2::boolean(false);
        
        assert!(t.is_boolean());
        assert!(f.is_boolean());
        assert_eq!(t.as_boolean(), Some(true));
        assert_eq!(f.as_boolean(), Some(false));
        assert!(t.is_truthy());
        assert!(f.is_falsy());
    }

    #[test]
    fn test_integer() {
        let v = LuaValueV2::integer(42);
        assert!(v.is_integer());
        assert!(v.is_number());
        assert_eq!(v.as_integer(), Some(42));
        
        // Test negative
        let neg = LuaValueV2::integer(-100);
        assert_eq!(neg.as_integer(), Some(-100));
        
        // Test i64 max
        let max = LuaValueV2::integer(i64::MAX);
        assert_eq!(max.as_integer(), Some(i64::MAX));
    }

    #[test]
    fn test_float() {
        let v = LuaValueV2::float(3.14);
        assert!(v.is_float());
        assert!(v.is_number());
        assert!((v.as_float().unwrap() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_table_id() {
        let v = LuaValueV2::table(TableId(123));
        assert!(v.is_table());
        assert_eq!(v.as_table_id(), Some(TableId(123)));
    }

    #[test]
    fn test_equality() {
        assert_eq!(LuaValueV2::nil(), LuaValueV2::nil());
        assert_eq!(LuaValueV2::integer(42), LuaValueV2::integer(42));
        assert_ne!(LuaValueV2::integer(42), LuaValueV2::integer(43));
        assert_eq!(LuaValueV2::table(TableId(1)), LuaValueV2::table(TableId(1)));
        assert_ne!(LuaValueV2::table(TableId(1)), LuaValueV2::table(TableId(2)));
    }
}
