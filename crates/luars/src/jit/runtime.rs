use cranelift_codegen::settings;
use cranelift_jit::{JITBuilder, JITModule};

use crate::gc::gc_object::GcTable;
use crate::lua_value::{LuaValue, Value};

thread_local! {
    static JIT_MODULE: std::cell::RefCell<JITModule> =
        std::cell::RefCell::new(create_module());
}

// ── JIT helper functions ────────────────────────────────────────────────────
// Called from compiled trace code via Cranelift function calls.
// Each takes raw i64 arguments matching the Cranelift ABI.

/// Read `t[key]` (integer key). Returns raw value bits (i64).
/// If the key is absent, returns 0 (nil value bits).
unsafe extern "C" fn jit_tab_geti(gc_table_ptr: i64, key: i64) -> i64 {
    let gc_table = unsafe { &*(gc_table_ptr as *const GcTable) };
    match gc_table.data.raw_geti(key) {
        Some(val) => unsafe { val.value.i },
        None => 0,
    }
}

/// Write `t[key] = val` (integer key).
/// `value` is raw value bits, `tag` is the type tag.
unsafe extern "C" fn jit_tab_seti(gc_table_ptr: i64, key: i64, value: i64, tag: i64) {
    let gc_table = unsafe { &mut *(gc_table_ptr as *mut GcTable) };
    let lv = LuaValue {
        value: Value { i: value },
        tt: tag as u8,
    };
    gc_table.data.raw_seti(key, lv);
}

/// Read `t[key]` (short string key). `key_ptr` is the raw LuaValue.value.i
/// of the interned string constant. Returns raw value bits (i64).
unsafe extern "C" fn jit_tab_gets(gc_table_ptr: i64, key_ptr: i64) -> i64 {
    let gc_table = unsafe { &*(gc_table_ptr as *const GcTable) };
    // Build a LuaValue with the short-string tag so get_shortstr_fast works.
    let key_lv = LuaValue {
        value: Value { i: key_ptr },
        tt: 0x44, // LUA_VSHRSTR
    };
    match gc_table.data.impl_table.get_shortstr_fast(&key_lv) {
        Some(val) => unsafe { val.value.i },
        None => 0,
    }
}

/// Write `t[key] = val` (short string key). Always succeeds --
/// tries fast_setfield, falls back to raw_set for new keys.
unsafe extern "C" fn jit_tab_sets(gc_table_ptr: i64, key_ptr: i64, value: i64, tag: i64) {
    let gc_table = unsafe { &mut *(gc_table_ptr as *mut GcTable) };
    let key_lv = LuaValue {
        value: Value { i: key_ptr },
        tt: 0x44, // LUA_VSHRSTR
    };
    let val_lv = LuaValue {
        value: Value { i: value },
        tt: tag as u8,
    };
    // fast path: update existing key
    if !gc_table.data.impl_table.fast_setfield(&key_lv, val_lv) {
        // slow path: new key insertion
        gc_table.data.raw_set(&key_lv, val_lv);
    }
}

/// Table length `#t`. Returns length as i64.
unsafe extern "C" fn jit_tab_len(gc_table_ptr: i64) -> i64 {
    let gc_table = unsafe { &*(gc_table_ptr as *const GcTable) };
    gc_table.data.len() as i64
}

fn create_module() -> JITModule {
    // Use default Cranelift settings -- the JIT backend already defaults to
    // generating fast code; no need to set opt_level explicitly.
    let isa = cranelift_native::builder()
        .expect("host ISA not supported by Cranelift")
        .finish(settings::Flags::new(settings::builder()))
        .expect("failed to create Cranelift ISA");

    let mut builder = JITBuilder::with_isa(
        isa,
        cranelift_module::default_libcall_names(),
    );

    // Register C `pow` so traces can compile PowFloat.
    unsafe extern "C" { fn pow(x: f64, y: f64) -> f64; }
    builder.symbol("pow", pow as *const u8);

    // Register table access helpers for trace JIT.
    builder.symbol("jit_tab_geti", jit_tab_geti as *const u8);
    builder.symbol("jit_tab_seti", jit_tab_seti as *const u8);
    builder.symbol("jit_tab_gets", jit_tab_gets as *const u8);
    builder.symbol("jit_tab_sets", jit_tab_sets as *const u8);
    builder.symbol("jit_tab_len", jit_tab_len as *const u8);

    JITModule::new(builder)
}

/// Run `f` with exclusive access to the thread-local `JITModule`.
pub fn with_module<F, R>(f: F) -> R
where
    F: FnOnce(&mut JITModule) -> R,
{
    JIT_MODULE.with(|cell| f(&mut cell.borrow_mut()))
}
