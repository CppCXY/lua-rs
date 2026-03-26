use std::{cell::RefCell, collections::HashMap, path::PathBuf, time::SystemTime};

use crate::{ProtoPtr, compiler::LuaLanguageLevel};

#[derive(Clone)]
pub struct SharedFileProtoEntry {
    pub proto: ProtoPtr,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub version: LuaLanguageLevel,
}

thread_local! {
    pub static SHARED_FILE_PROTO_CACHE: RefCell<HashMap<PathBuf, SharedFileProtoEntry>> =
        RefCell::new(HashMap::new());
}
