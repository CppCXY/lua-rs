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
