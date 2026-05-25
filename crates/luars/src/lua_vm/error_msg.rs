use crate::LuaValue;

#[derive(Debug, Default)]
pub enum ErrorMsg {
    #[default]
    None,
    Msg(String),
    Object(LuaValue),
}