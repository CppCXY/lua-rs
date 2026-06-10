//! Liveness token for borrowed userdata references.
//!
//! [`RefAliveToken`] is an `Rc<Cell<bool>>` wrapper that tracks whether the
//! backing data for a [`Borrowed`](crate::LuaUserdata) userdata is still alive.
//!
//! # Lifecycle
//!
//! 1. **Owned userdata** (`LuaUserdata::new`): creates an alive token on construction.
//!    When the userdata is GC-collected, the token flips to `false`.
//! 2. **Borrowed userdata** (`LuaUserdata::from_ptr`): shares a token created by
//!    the parent (or a scope). When the parent is dropped, all borrowed children
//!    see `is_alive() == false`.
//! 3. **Scope** (`Scope::create_userdata_ref`): shares the scope's token. When the
//!    scope ends, all scoped userdata become expired.
//!
//! # Usage in structs
//!
//! Mark a field with `#[lua(ref)]` to enable `IntoLua for &T` / `IntoLua for &mut T`:
//!
//! ```ignore
//! #[derive(LuaUserData)]
//! struct Entity {
//!     pub name: String,
//!     pub pos: Position,
//!
//!     alive: RefAliveToken,  // enables &Entity → Lua conversion
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
