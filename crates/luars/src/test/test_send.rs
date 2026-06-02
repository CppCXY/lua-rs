//! Compile-time proof that `send` feature makes core types `Send + Sync`.
//!
//! This module only compiles when `--features send` is active.
//! The test functions use static assertions — if any type fails the
//! `Send` or `Sync` bound, the test won't compile.

use crate::lua_vm::{
    GlobalState, LuaAnyRef, LuaFunctionRef, LuaState, LuaStringRef, LuaTableRef, UserDataRef,
};
use crate::{Lua, LuaValue, RefAliveToken};

// Static assertion helpers
fn _assert_send<T: Send>() {}
fn _assert_sync<T: Sync>() {}
fn _assert_send_sync<T: Send + Sync>() {}

// ==================== Core VM types ====================

#[test]
fn global_state_is_send_sync() {
    _assert_send_sync::<GlobalState>();
}

#[test]
fn lua_state_is_send_sync() {
    _assert_send_sync::<LuaState>();
}

#[test]
fn lua_is_send_sync() {
    _assert_send_sync::<Lua>();
}

// ==================== Value types ====================

#[test]
fn lua_value_is_send_sync() {
    _assert_send_sync::<LuaValue>();
}

#[test]
fn ref_alive_token_is_send_sync() {
    _assert_send_sync::<RefAliveToken>();
}

// ==================== Reference types ====================

#[test]
fn user_data_ref_is_send_sync() {
    _assert_send_sync::<UserDataRef<crate::lua_value::userdata_trait::OpaqueUserData<i32>>>();
}

#[test]
fn lua_table_ref_is_send_sync() {
    _assert_send_sync::<LuaTableRef>();
}

#[test]
fn lua_function_ref_is_send_sync() {
    _assert_send_sync::<LuaFunctionRef>();
}

#[test]
fn lua_any_ref_is_send_sync() {
    _assert_send_sync::<LuaAnyRef>();
}

#[test]
fn lua_string_ref_is_send_sync() {
    _assert_send_sync::<LuaStringRef>();
}

// ==================== Move-semantics check ====================

/// Prove that `Lua` can actually be moved to another thread (Send),
/// not just shared (&Sync).
#[test]
fn lua_can_be_sent_to_thread() {
    use std::thread;

    let lua = Lua::new(crate::SafeOption::default());
    let handle = thread::spawn(move || {
        // Lua moved into this thread — proves Send
        let _ = lua;
    });
    handle.join().unwrap();
}

/// Prove that `&Lua` can be shared across threads (Sync).
#[test]
fn lua_ref_can_be_shared() {
    use std::sync::Arc;
    use std::thread;

    let lua = Lua::new(crate::SafeOption::default());
    let shared = Arc::new(lua);

    let h1 = {
        let shared = shared.clone();
        thread::spawn(move || {
            let _ = &*shared; // &Lua — proves Sync
        })
    };
    let h2 = {
        let shared = shared.clone();
        thread::spawn(move || {
            let _ = &*shared;
        })
    };

    h1.join().unwrap();
    h2.join().unwrap();
}
