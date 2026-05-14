use std::{cell::RefCell, collections::HashMap, path::PathBuf, time::SystemTime};

use crate::{
    compiler::LuaLanguageLevel,
    gc::{GC, ObjectAllocator, ProtoPtr},
    lua_vm::const_string::ConstString,
};

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
    static SHARED_CONST_STRINGS: RefCell<Option<ConstString>> = const { RefCell::new(None) };
}

pub fn get_or_init_const_strings(allocator: &mut ObjectAllocator, gc: &mut GC) -> ConstString {
    SHARED_CONST_STRINGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let const_strings = if let Some(existing) = *cache {
            existing
        } else {
            let mut created = ConstString::new(allocator, gc);
            created.share_all();
            *cache = Some(created);
            created
        };

        const_strings.attach_to_gc(gc);
        const_strings
    })
}
