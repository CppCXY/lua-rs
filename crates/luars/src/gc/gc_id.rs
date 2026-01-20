#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcObjectKind {
    String = 0,
    Table = 1,
    Function = 2,
    Upvalue = 3,
    Thread = 4,
    Userdata = 5,
    Binary = 6,
}

/// Unified GC object identifier
/// Layout: [type: 3 bits][index: 29 bits]
/// Supports up to 536 million objects per type
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GcId {
    kind: GcObjectKind,
    index: u32,
}

impl GcId {
    pub fn new(kind: GcObjectKind, index: u32) -> Self {
        Self { kind, index }
    }

    #[inline(always)]
    pub fn gc_type(self) -> GcObjectKind {
        self.kind
    }

    #[inline(always)]
    pub fn index(self) -> u32 {
        self.index
    }

    pub fn main_id() -> Self {
        Self {
            kind: GcObjectKind::Thread,
            index: u32::MAX,
        }
    }

    pub fn is_main(self) -> bool {
        self.kind == GcObjectKind::Thread && self.index == u32::MAX
    }
}
