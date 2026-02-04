#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcObjectKind {
    String = 0,
    Table = 1,
    Function = 2,
    CClosure = 3,
    Upvalue = 4,
    Thread = 5,
    Userdata = 6,
    Binary = 7,
}
