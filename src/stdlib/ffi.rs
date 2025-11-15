// FFI library for Lua

use crate::ffi;
use crate::lib_registry::{LibraryModule, LibraryEntry};
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaVM;

pub fn create_ffi_lib() -> LibraryModule {
    LibraryModule {
        name: "ffi",
        entries: vec![
            ("cdef", LibraryEntry::Function(ffi_cdef_wrapper)),
            ("new", LibraryEntry::Function(ffi_new_wrapper)),
            ("typeof", LibraryEntry::Function(ffi_typeof_wrapper)),
            ("cast", LibraryEntry::Function(ffi_cast_wrapper)),
            ("sizeof", LibraryEntry::Function(ffi_sizeof_wrapper)),
            ("alignof", LibraryEntry::Function(ffi_alignof_wrapper)),
            ("offsetof", LibraryEntry::Function(ffi_offsetof_wrapper)),
            ("istype", LibraryEntry::Function(ffi_istype_wrapper)),
            ("load", LibraryEntry::Function(ffi_load_wrapper)),
            ("string", LibraryEntry::Function(ffi_string_wrapper)),
            ("copy", LibraryEntry::Function(ffi_copy_wrapper)),
            ("fill", LibraryEntry::Function(ffi_fill_wrapper)),
            ("C", LibraryEntry::Value(create_ffi_c_namespace)),
        ],
    }
}

// Create ffi.C namespace table
fn create_ffi_c_namespace(vm: &mut LuaVM) -> LuaValue {
    let table = vm.create_table();
    
    // Set __index metamethod for lazy symbol loading
    let meta = vm.create_table();
    let index_key = vm.create_string("__index".to_string());
    meta.borrow_mut().raw_set(
        LuaValue::from_string_rc(index_key),
        LuaValue::cfunction(ffi_c_index_wrapper)
    );
    
    table.borrow_mut().set_metatable(Some(meta));
    LuaValue::from_table_rc(table)
}

fn ffi_c_index_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_c_index(vm)
}

// Wrapper functions that call the actual FFI implementations

fn ffi_cdef_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_cdef(vm)
}

fn ffi_new_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_new(vm)
}

fn ffi_typeof_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_typeof(vm)
}

fn ffi_cast_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_cast(vm)
}

fn ffi_sizeof_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_sizeof(vm)
}

fn ffi_alignof_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_alignof(vm)
}

fn ffi_offsetof_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_offsetof(vm)
}

fn ffi_istype_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_istype(vm)
}

fn ffi_load_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_load(vm)
}

fn ffi_string_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_string(vm)
}

fn ffi_copy_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_copy(vm)
}

fn ffi_fill_wrapper(vm: &mut LuaVM) -> Result<MultiValue, String> {
    ffi::ffi_fill(vm)
}
