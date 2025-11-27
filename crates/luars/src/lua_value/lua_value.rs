// LuaValue - Simplified design without pointer caching
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

use crate::gc::{FunctionId, StringId, TableId, ThreadId, UpvalueId, UserdataId};
use crate::lua_value::CFunction;

// Type tags (high 16 bits of tag field)
pub const TAG_NIL: u64 = 0x0000_0000_0000_0000;
pub const TAG_FALSE: u64 = 0x0001_0000_0000_0000;
pub const TAG_TRUE: u64 = 0x0002_0000_0000_0000;
pub const TAG_INTEGER: u64 = 0x0003_0000_0000_0000;
pub const TAG_FLOAT: u64 = 0x0004_0000_0000_0000;
pub const TAG_STRING: u64 = 0x0005_0000_0000_0000;
pub const TAG_TABLE: u64 = 0x0006_0000_0000_0000;
pub const TAG_FUNCTION: u64 = 0x0007_0000_0000_0000;
pub const TAG_CFUNCTION: u64 = 0x0008_0000_0000_0000;
pub const TAG_USERDATA: u64 = 0x0009_0000_0000_0000;
pub const TAG_UPVALUE: u64 = 0x000A_0000_0000_0000;
pub const TAG_THREAD: u64 = 0x000B_0000_0000_0000;

pub const TAG_MASK: u64 = 0xFFFF_0000_0000_0000;
pub const ID_MASK: u64 = 0x0000_0000_FFFF_FFFF;

// Compatibility constants (aliases)
pub const TYPE_MASK: u64 = TAG_MASK;
pub const TAG_BOOLEAN: u64 = TAG_TRUE; // Use TAG_TRUE or TAG_FALSE
pub const VALUE_NIL: u64 = TAG_NIL;
pub const VALUE_TRUE: u64 = TAG_TRUE;
pub const VALUE_FALSE: u64 = TAG_FALSE;
pub const NAN_BASE: u64 = TAG_INTEGER; // Not really used in new design

/// LuaValue - no pointer caching, all GC objects accessed via ID
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuaValue {
    pub(crate) primary: u64, // tag: type tag (high 16 bits) + object ID (low 32 bits)
    pub(crate) secondary: u64, // data: i64/f64/cfunc pointer (for inline types)
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

impl LuaValue {
    // ============ Constructors ============

    #[inline(always)]
    pub const fn nil() -> Self {
        Self {
            primary: TAG_NIL,
            secondary: 0,
        }
    }

    #[inline(always)]
    pub const fn boolean(b: bool) -> Self {
        Self {
            primary: if b { TAG_TRUE } else { TAG_FALSE },
            secondary: 0,
        }
    }

    #[inline(always)]
    pub const fn integer(i: i64) -> Self {
        Self {
            primary: TAG_INTEGER,
            secondary: i as u64,
        }
    }

    #[inline(always)]
    pub fn float(f: f64) -> Self {
        Self {
            primary: TAG_FLOAT,
            secondary: f.to_bits(),
        }
    }

    /// Alias for float()
    #[inline(always)]
    pub fn number(n: f64) -> Self {
        Self::float(n)
    }

    #[inline(always)]
    pub fn string(id: StringId) -> Self {
        Self {
            primary: TAG_STRING | (id.0 as u64),
            secondary: 0,
        }
    }

    #[inline(always)]
    pub fn table(id: TableId) -> Self {
        Self {
            primary: TAG_TABLE | (id.0 as u64),
            secondary: 0,
        }
    }

    #[inline(always)]
    pub fn function(id: FunctionId) -> Self {
        Self {
            primary: TAG_FUNCTION | (id.0 as u64),
            secondary: 0,
        }
    }

    /// Alias for function() - for compatibility
    #[inline(always)]
    pub fn function_id(id: FunctionId) -> Self {
        Self::function(id)
    }

    #[inline(always)]
    pub fn cfunction(f: CFunction) -> Self {
        Self {
            primary: TAG_CFUNCTION,
            secondary: f as usize as u64,
        }
    }

    #[inline(always)]
    pub fn userdata(id: UserdataId) -> Self {
        Self {
            primary: TAG_USERDATA | (id.0 as u64),
            secondary: 0,
        }
    }

    #[inline(always)]
    pub fn upvalue(id: UpvalueId) -> Self {
        Self {
            primary: TAG_UPVALUE | (id.0 as u64),
            secondary: 0,
        }
    }

    #[inline(always)]
    pub fn thread(id: ThreadId) -> Self {
        Self {
            primary: TAG_THREAD | (id.0 as u64),
            secondary: 0,
        }
    }

    // ============ Type checking ============

    #[inline(always)]
    pub fn kind(&self) -> LuaValueKind {
        match self.primary & TAG_MASK {
            TAG_NIL => LuaValueKind::Nil,
            TAG_FALSE | TAG_TRUE => LuaValueKind::Boolean,
            TAG_INTEGER => LuaValueKind::Integer,
            TAG_FLOAT => LuaValueKind::Float,
            TAG_STRING => LuaValueKind::String,
            TAG_TABLE => LuaValueKind::Table,
            TAG_FUNCTION => LuaValueKind::Function,
            TAG_CFUNCTION => LuaValueKind::CFunction,
            TAG_USERDATA => LuaValueKind::Userdata,
            TAG_THREAD => LuaValueKind::Thread,
            _ => LuaValueKind::Nil,
        }
    }

    #[inline(always)]
    pub fn is_nil(&self) -> bool {
        self.primary == TAG_NIL
    }

    #[inline(always)]
    pub fn is_boolean(&self) -> bool {
        matches!(self.primary & TAG_MASK, TAG_FALSE | TAG_TRUE)
    }

    #[inline(always)]
    pub fn is_integer(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_INTEGER
    }

    #[inline(always)]
    pub fn is_float(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_FLOAT
    }

    #[inline(always)]
    pub fn is_number(&self) -> bool {
        matches!(self.primary & TAG_MASK, TAG_INTEGER | TAG_FLOAT)
    }

    #[inline(always)]
    pub fn is_string(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_STRING
    }

    #[inline(always)]
    pub fn is_table(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_TABLE
    }

    #[inline(always)]
    pub fn is_function(&self) -> bool {
        matches!(self.primary & TAG_MASK, TAG_FUNCTION | TAG_CFUNCTION)
    }

    #[inline(always)]
    pub fn is_lua_function(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_FUNCTION
    }

    #[inline(always)]
    pub fn is_cfunction(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_CFUNCTION
    }

    #[inline(always)]
    pub fn is_userdata(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_USERDATA
    }

    #[inline(always)]
    pub fn is_thread(&self) -> bool {
        (self.primary & TAG_MASK) == TAG_THREAD
    }

    #[inline(always)]
    pub fn is_callable(&self) -> bool {
        self.is_function() || self.is_cfunction()
    }

    // ============ Value extraction ============

    #[inline(always)]
    pub fn as_boolean(&self) -> Option<bool> {
        match self.primary {
            TAG_TRUE => Some(true),
            TAG_FALSE => Some(false),
            _ => None,
        }
    }

    /// Alias for as_boolean
    #[inline(always)]
    pub fn as_bool(&self) -> Option<bool> {
        self.as_boolean()
    }

    #[inline(always)]
    pub fn as_integer(&self) -> Option<i64> {
        if (self.primary & TAG_MASK) == TAG_INTEGER {
            Some(self.secondary as i64)
        } else if (self.primary & TAG_MASK) == TAG_FLOAT {
            // Lua 5.4 semantics: floats with zero fraction are integers
            let f = f64::from_bits(self.secondary);
            if f.fract() == 0.0 && f.is_finite() {
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
        if (self.primary & TAG_MASK) == TAG_FLOAT {
            Some(f64::from_bits(self.secondary))
        } else if (self.primary & TAG_MASK) == TAG_INTEGER {
            Some(self.secondary as i64 as f64)
        } else {
            None
        }
    }

    /// Get as number (integer or float)
    #[inline(always)]
    pub fn as_number(&self) -> Option<f64> {
        match self.primary & TAG_MASK {
            TAG_INTEGER => Some(self.secondary as i64 as f64),
            TAG_FLOAT => Some(f64::from_bits(self.secondary)),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_string_id(&self) -> Option<StringId> {
        if (self.primary & TAG_MASK) == TAG_STRING {
            Some(StringId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_table_id(&self) -> Option<TableId> {
        if (self.primary & TAG_MASK) == TAG_TABLE {
            Some(TableId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_function_id(&self) -> Option<FunctionId> {
        if (self.primary & TAG_MASK) == TAG_FUNCTION {
            Some(FunctionId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_cfunction(&self) -> Option<CFunction> {
        if (self.primary & TAG_MASK) == TAG_CFUNCTION {
            // SAFETY: We stored a valid function pointer
            Some(unsafe { std::mem::transmute(self.secondary as usize) })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_userdata_id(&self) -> Option<UserdataId> {
        if (self.primary & TAG_MASK) == TAG_USERDATA {
            Some(UserdataId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_upvalue_id(&self) -> Option<UpvalueId> {
        if (self.primary & TAG_MASK) == TAG_UPVALUE {
            Some(UpvalueId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_thread_id(&self) -> Option<ThreadId> {
        if (self.primary & TAG_MASK) == TAG_THREAD {
            Some(ThreadId((self.primary & ID_MASK) as u32))
        } else {
            None
        }
    }

    // ============ Truthiness ============

    /// Lua truthiness: only nil and false are falsy
    #[inline(always)]
    pub fn is_truthy(&self) -> bool {
        !matches!(self.primary, TAG_NIL | TAG_FALSE)
    }

    #[inline(always)]
    pub fn is_falsy(&self) -> bool {
        matches!(self.primary, TAG_NIL | TAG_FALSE)
    }

    // ============ Object ID extraction (for GC) ============

    /// Get object ID if this is a GC object
    #[inline(always)]
    pub fn gc_object_id(&self) -> Option<(u8, u32)> {
        let type_tag = ((self.primary & TAG_MASK) >> 48) as u8;
        match type_tag {
            5..=11 => Some((type_tag, (self.primary & ID_MASK) as u32)),
            _ => None,
        }
    }

    // ============ Equality ============

    /// Raw equality (no metamethods)
    #[inline(always)]
    pub fn raw_equal(&self, other: &Self) -> bool {
        // For most types, both tag and data must match
        // Special case: NaN != NaN for floats
        if (self.primary & TAG_MASK) == TAG_FLOAT && (other.primary & TAG_MASK) == TAG_FLOAT {
            let a = f64::from_bits(self.secondary);
            let b = f64::from_bits(other.secondary);
            a == b // This handles NaN correctly
        } else {
            self.primary == other.primary && self.secondary == other.secondary
        }
    }

    // ============ Type name ============

    pub fn type_name(&self) -> &'static str {
        match self.kind() {
            LuaValueKind::Nil => "nil",
            LuaValueKind::Boolean => "boolean",
            LuaValueKind::Integer => "number",
            LuaValueKind::Float => "number",
            LuaValueKind::String => "string",
            LuaValueKind::Table => "table",
            LuaValueKind::Function => "function",
            LuaValueKind::CFunction => "function",
            LuaValueKind::Userdata => "userdata",
            LuaValueKind::Thread => "thread",
        }
    }

    /// Raw primary value access (for advanced use cases)
    #[inline(always)]
    pub fn primary(&self) -> u64 {
        self.primary
    }

    /// Raw secondary value access (for advanced use cases)
    #[inline(always)]
    pub fn secondary(&self) -> u64 {
        self.secondary
    }
}

impl Default for LuaValue {
    fn default() -> Self {
        Self::nil()
    }
}

impl std::fmt::Debug for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            LuaValueKind::Nil => write!(f, "nil"),
            LuaValueKind::Boolean => write!(f, "{}", self.primary == TAG_TRUE),
            LuaValueKind::Integer => write!(f, "{}", self.secondary as i64),
            LuaValueKind::Float => write!(f, "{}", f64::from_bits(self.secondary)),
            LuaValueKind::String => write!(f, "string({})", (self.primary & ID_MASK)),
            LuaValueKind::Table => write!(f, "table({})", (self.primary & ID_MASK)),
            LuaValueKind::Function => write!(f, "function({})", (self.primary & ID_MASK)),
            LuaValueKind::CFunction => write!(f, "cfunction({:#x})", self.secondary),
            LuaValueKind::Userdata => write!(f, "userdata({})", (self.primary & ID_MASK)),
            LuaValueKind::Thread => write!(f, "thread({})", (self.primary & ID_MASK)),
        }
    }
}

impl std::fmt::Display for LuaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            LuaValueKind::Nil => write!(f, "nil"),
            LuaValueKind::Boolean => write!(f, "{}", self.primary == TAG_TRUE),
            LuaValueKind::Integer => write!(f, "{}", self.secondary as i64),
            LuaValueKind::Float => {
                let n = f64::from_bits(self.secondary);
                if n.floor() == n && n.abs() < 1e14 {
                    write!(f, "{:.0}", n)
                } else {
                    write!(f, "{}", n)
                }
            }
            LuaValueKind::String => write!(f, "string({})", (self.primary & ID_MASK)),
            LuaValueKind::Table => write!(f, "table: {:x}", (self.primary & ID_MASK)),
            LuaValueKind::Function => write!(f, "function: {:x}", (self.primary & ID_MASK)),
            LuaValueKind::CFunction => write!(f, "function: {:x}", self.secondary),
            LuaValueKind::Userdata => write!(f, "userdata: {:x}", (self.primary & ID_MASK)),
            LuaValueKind::Thread => write!(f, "thread: {:x}", (self.primary & ID_MASK)),
        }
    }
}

impl PartialEq for LuaValue {
    fn eq(&self, other: &Self) -> bool {
        self.raw_equal(other)
    }
}

impl Eq for LuaValue {}

impl std::hash::Hash for LuaValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash both tag and data
        self.primary.hash(state);
        self.secondary.hash(state);
    }
}

impl PartialOrd for LuaValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;
        let kind_a = self.kind();
        let kind_b = other.kind();

        match kind_a.cmp(&kind_b) {
            Ordering::Equal => match kind_a {
                LuaValueKind::Nil => Some(Ordering::Equal),
                LuaValueKind::Boolean => self.as_boolean().partial_cmp(&other.as_boolean()),
                LuaValueKind::Integer => self.as_integer().partial_cmp(&other.as_integer()),
                LuaValueKind::Float => self.as_float().partial_cmp(&other.as_float()),
                LuaValueKind::String => {
                    // Compare by ID for now (proper string comparison needs ObjectPool)
                    let id_a = (self.primary & ID_MASK) as u32;
                    let id_b = (other.primary & ID_MASK) as u32;
                    id_a.partial_cmp(&id_b)
                }
                _ => {
                    let id_a = (self.primary & ID_MASK) as u32;
                    let id_b = (other.primary & ID_MASK) as u32;
                    id_a.partial_cmp(&id_b)
                }
            },
            ord => Some(ord),
        }
    }
}

impl Ord for LuaValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size() {
        assert_eq!(std::mem::size_of::<LuaValue>(), 16);
    }

    #[test]
    fn test_nil() {
        let v = LuaValue::nil();
        assert!(v.is_nil());
        assert!(v.is_falsy());
    }

    #[test]
    fn test_boolean() {
        let t = LuaValue::boolean(true);
        let f = LuaValue::boolean(false);

        assert!(t.is_boolean());
        assert!(f.is_boolean());
        assert_eq!(t.as_boolean(), Some(true));
        assert_eq!(f.as_boolean(), Some(false));
        assert!(t.is_truthy());
        assert!(f.is_falsy());
    }

    #[test]
    fn test_integer() {
        let v = LuaValue::integer(42);
        assert!(v.is_integer());
        assert!(v.is_number());
        assert_eq!(v.as_integer(), Some(42));

        // Test negative
        let neg = LuaValue::integer(-100);
        assert_eq!(neg.as_integer(), Some(-100));

        // Test i64 max
        let max = LuaValue::integer(i64::MAX);
        assert_eq!(max.as_integer(), Some(i64::MAX));
    }

    #[test]
    fn test_float() {
        let v = LuaValue::float(3.14);
        assert!(v.is_float());
        assert!(v.is_number());
        assert!((v.as_float().unwrap() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_table_id() {
        let v = LuaValue::table(TableId(123));
        assert!(v.is_table());
        assert_eq!(v.as_table_id(), Some(TableId(123)));
    }

    #[test]
    fn test_equality() {
        assert_eq!(LuaValue::nil(), LuaValue::nil());
        assert_eq!(LuaValue::integer(42), LuaValue::integer(42));
        assert_ne!(LuaValue::integer(42), LuaValue::integer(43));
        assert_eq!(LuaValue::table(TableId(1)), LuaValue::table(TableId(1)));
        assert_ne!(LuaValue::table(TableId(1)), LuaValue::table(TableId(2)));
    }
}
