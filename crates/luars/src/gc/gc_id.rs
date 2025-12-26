// ============ Object IDs ============
// All IDs are simple u32 indices - compact and efficient

/// String ID with embedded long/short flag
/// Layout: [is_long: 1 bit][index: 31 bits]
/// - Bit 31 (0x8000_0000): 1 = long string, 0 = short string
/// - Bits 0-30: actual index in string pool
/// Supports up to 2 billion strings per type (short/long)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct StringId(u32);

impl StringId {
    const LONG_STRING_BIT: u32 = 0x8000_0000;
    const INDEX_MASK: u32 = 0x7FFF_FFFF;

    /// Create a short string ID
    #[inline(always)]
    pub const fn short(index: u32) -> Self {
        debug_assert!(index <= Self::INDEX_MASK);
        Self(index)
    }

    /// Create a long string ID
    #[inline(always)]
    pub const fn long(index: u32) -> Self {
        debug_assert!(index <= Self::INDEX_MASK);
        Self(index | Self::LONG_STRING_BIT)
    }

    /// Check if this is a long string
    #[inline(always)]
    pub const fn is_long(self) -> bool {
        (self.0 & Self::LONG_STRING_BIT) != 0
    }

    /// Check if this is a short string
    #[inline(always)]
    pub const fn is_short(self) -> bool {
        !self.is_long()
    }

    /// Get the actual index (without the flag bit)
    #[inline(always)]
    pub const fn index(self) -> u32 {
        self.0 & Self::INDEX_MASK
    }

    #[inline(always)]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Get the raw u32 value (with flag bit)
    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl Default for StringId {
    fn default() -> Self {
        Self::short(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct TableId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct FunctionId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct UpvalueId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct UserdataId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct ThreadId(pub u32);

/// Object type tags (3 bits, supports up to 8 types)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcObjectType {
    String = 0,
    Table = 1,
    Function = 2,
    Upvalue = 3,
    Thread = 4,
    Userdata = 5,
}

/// Unified GC object identifier
/// Layout: [type: 3 bits][index: 29 bits]
/// Supports up to 536 million objects per type
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GcId {
    StringId(StringId),
    TableId(TableId),
    FunctionId(FunctionId),
    UpvalueId(UpvalueId),
    ThreadId(ThreadId),
    UserdataId(UserdataId),
}

impl GcId {
    #[inline(always)]
    pub fn gc_type(self) -> GcObjectType {
        match self {
            GcId::StringId(_) => GcObjectType::String,
            GcId::TableId(_) => GcObjectType::Table,
            GcId::FunctionId(_) => GcObjectType::Function,
            GcId::UpvalueId(_) => GcObjectType::Upvalue,
            GcId::ThreadId(_) => GcObjectType::Thread,
            GcId::UserdataId(_) => GcObjectType::Userdata,
        }
    }

    #[inline(always)]
    pub fn index(self) -> u32 {
        match self {
            GcId::StringId(StringId(id)) => id,
            GcId::TableId(TableId(id)) => id,
            GcId::FunctionId(FunctionId(id)) => id,
            GcId::UpvalueId(UpvalueId(id)) => id,
            GcId::ThreadId(ThreadId(id)) => id,
            GcId::UserdataId(UserdataId(id)) => id,
        }
    }
}
