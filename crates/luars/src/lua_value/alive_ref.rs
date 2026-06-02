//! Sub-reference system for Lua userdata.
//!
//! Enables userdata methods to return references to internal data (not copies),
//! and struct fields of userdata type to yield sub-references automatically.
//!
//! # How it works
//!
//! 1. Every [`LuaUserdata`](crate::LuaUserdata) carries a `sub_guard: Rc<Cell<bool>>`.
//! 2. When an owned userdata is GC-collected, its [`Drop`] flips the guard to `false`.
//! 3. [`SubRefToken`] is a clone of the guard — each sub-reference holds one.
//! 4. [`SubRef<T>`] wraps a raw pointer to the sub-object plus a token.
//!    Every access checks `token.is_alive()` first.
//! 5. The `#[derive(LuaUserData)]` macro generates `UdValue::SubRef(ptr)` for
//!    non-primitive fields; the VM layer wraps them with the parent's token.
//!
//! # Example
//!
//! ```ignore
//! #[derive(LuaUserData)]
//! struct Player {
//!     pub hp: i64,
//!     pub pos: Position,  // non-primitive → auto sub-ref
//! }
//!
//! #[lua_methods]
//! impl Player {
//!     pub fn get_pos(&self) -> &Position { &self.pos }
//! }
//! ```

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

// ============================================================================
// RefAliveToken — liveness tracking
// ============================================================================

/// A liveness token tied to a parent [`LuaUserdata`](crate::LuaUserdata).
///
/// Each sub-reference holds one token. When the parent userdata is GC-collected
/// (only for owned storage), the token becomes expired and all sub-references
/// will return errors on access.
///
/// `!Send + !Sync` (contains `Rc`).
#[derive(Clone)]
pub struct RefAliveToken {
    inner: Rc<Cell<bool>>,
}

impl RefAliveToken {
    /// Create from an existing `Rc<Cell<bool>>`. Used by `LuaUserdata`.
    #[inline]
    pub fn from_inner(inner: Rc<Cell<bool>>) -> Self {
        Self { inner }
    }

    /// Create a permanently dead token. Sub-references created with this
    /// token will always appear expired. Used for borrowed userdata.
    #[inline]
    pub fn dead() -> Self {
        Self {
            inner: Rc::new(Cell::new(false)),
        }
    }

    /// Check whether the parent userdata is still alive.
    #[inline]
    pub fn is_alive(&self) -> bool {
        self.inner.get()
    }

    #[inline]
    pub fn set(&self, value: bool) {
        self.inner.set(value);
    }
}

impl fmt::Debug for RefAliveToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefAliveToken")
            .field("alive", &self.inner.get())
            .finish()
    }
}

impl Default for RefAliveToken {
    /// Create a new token in the alive state. Used by `LuaUserdata::new`.
    #[inline]
    fn default() -> Self {
        Self {
            inner: Rc::new(Cell::new(true)),
        }
    }
}
