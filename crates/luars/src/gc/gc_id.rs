// ============ Object IDs ============
// All IDs are simple u32 indices - compact and efficient

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct StringId(pub u32);

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
pub enum GcType {
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
    pub fn gc_type(self) -> GcType {
        match self {
            GcId::StringId(_) => GcType::String,
            GcId::TableId(_) => GcType::Table,
            GcId::FunctionId(_) => GcType::Function,
            GcId::UpvalueId(_) => GcType::Upvalue,
            GcId::ThreadId(_) => GcType::Thread,
            GcId::UserdataId(_) => GcType::Userdata,
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
