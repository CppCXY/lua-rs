#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcObjectKind {
    String = 0,
    Table = 1,
    Function = 2,
    CClosure = 3,
    RClosure = 4,
    Upvalue = 5,
    Thread = 6,
    Userdata = 7,
    Proto = 8,
}

impl GcObjectKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::String),
            1 => Some(Self::Table),
            2 => Some(Self::Function),
            3 => Some(Self::CClosure),
            4 => Some(Self::RClosure),
            5 => Some(Self::Upvalue),
            6 => Some(Self::Thread),
            7 => Some(Self::Userdata),
            8 => Some(Self::Proto),
            _ => None,
        }
    }
}
