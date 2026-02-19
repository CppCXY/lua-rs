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
    Binary = 8,
}
