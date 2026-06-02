//! Userdata — GC-managed or borrowed Rust objects exposed to Lua.
//!
//! # Storage variants
//!
//! [`LuaUserdata`] stores data in one of two ways:
//!
//! | Variant | Ownership | Lifetime tracking |
//! |---|---|---|
//! | [`Owned`](UserdataStorage::Owned) | GC owns the data | Token flips on drop |
//! | [`Borrowed`](UserdataStorage::Borrowed) | External / sub-reference | Token shared with parent |
//!
//! # Sub-references
//!
//! When a method returns `&T` or a field of userdata type is accessed,
//! the result is a **borrowed** userdata — a `LuaUserdata` whose storage
//! variant is `Borrowed`, sharing the parent's [`RefAliveToken`].
//!
//! All accessor methods ([`get_trait`], [`get_trait_mut`], etc.) check
//! `is_alive()` before dereferencing. If the backing data has expired,
//! they return [`LuaError::ExpiredReference`].
//!
//! # Example
//!
//! ```ignore
//! #[derive(LuaUserData)]
//! struct Entity {
//!     pub name: String,
//!     pub pos: Position,        // non-primitive → sub-reference on access
//!     #[lua(skip)]
//!     #[lua(ref)]
//!     alive: RefAliveToken,     // enables IntoLua for &Entity
//! }
//! ```

use std::{any::Any, fmt};

use crate::lua_vm::lua_error::LuaError;
use crate::{LuaValue, RefAliveToken, UserDataTrait, gc::TablePtr};

/// Userdata storage — either owns the data or borrows it via raw pointer.
pub enum UserdataStorage {
    /// GC-managed: the data is owned and dropped when this userdata is collected.
    Owned(Box<dyn UserDataTrait>),
    /// Borrowed: a raw pointer to data with an external lifetime.
    /// Validity is tracked via the [`RefAliveToken`] in [`LuaUserdata`].
    Borrowed(*mut dyn UserDataTrait),
}

/// GC-managed userdata — the bridge between Rust types and Lua values.
///
/// Every `LuaUserdata` carries an [`RefAliveToken`]. For `Owned` storage,
/// the token starts alive and flips to `false` when the userdata is dropped.
/// All accessor methods return [`LuaError::ExpiredReference`] if the token
/// has expired.
pub struct LuaUserdata {
    data: UserdataStorage,
    metatable: TablePtr,
    alive_token: RefAliveToken,
}

impl LuaUserdata {
    // ==================== Constructors ====================

    /// Create a new **owned** userdata.
    pub fn new<T: UserDataTrait>(data: T) -> Self {
        LuaUserdata {
            data: UserdataStorage::Owned(Box::new(data)),
            metatable: TablePtr::null(),
            alive_token: RefAliveToken::default(),
        }
    }

    /// Create an owned userdata from an already-boxed trait object.
    pub fn from_boxed(data: Box<dyn UserDataTrait>) -> Self {
        LuaUserdata {
            data: UserdataStorage::Owned(data),
            metatable: TablePtr::null(),
            alive_token: RefAliveToken::default(),
        }
    }

    /// Create a **borrowed** userdata from a mutable reference + liveness token.
    pub fn from_ref<T: UserDataTrait>(reference: &mut T, token: RefAliveToken) -> Self {
        LuaUserdata {
            data: UserdataStorage::Borrowed(reference as *mut T as *mut dyn UserDataTrait),
            metatable: TablePtr::null(),
            alive_token: token,
        }
    }

    /// Create a borrowed userdata from a const pointer + liveness token.
    pub fn from_ptr<T: UserDataTrait>(ptr: *const T, token: RefAliveToken) -> Self {
        LuaUserdata {
            data: UserdataStorage::Borrowed(ptr as *mut T as *mut dyn UserDataTrait),
            metatable: TablePtr::null(),
            alive_token: token,
        }
    }

    /// Create a borrowed userdata from a raw trait object pointer.
    pub fn from_trait_ptr(ptr: *const (dyn UserDataTrait + 'static), token: RefAliveToken) -> Self {
        LuaUserdata {
            data: UserdataStorage::Borrowed(ptr as *mut (dyn UserDataTrait + 'static)),
            metatable: TablePtr::null(),
            alive_token: token,
        }
    }

    /// Create a new owned userdata with an initial metatable.
    pub fn with_metatable<T: UserDataTrait>(data: T, metatable: TablePtr) -> Self {
        LuaUserdata {
            data: UserdataStorage::Owned(Box::new(data)),
            metatable,
            alive_token: RefAliveToken::default(),
        }
    }

    // ==================== Liveness ====================

    /// Returns `true` if the backing data is still alive.
    #[inline]
    pub fn is_alive(&self) -> bool {
        match &self.data {
            UserdataStorage::Owned(_) => true,
            UserdataStorage::Borrowed(_) => self.alive_token.is_alive(),
        }
    }

    /// Clone the liveness token (for creating sub-references).
    #[inline]
    pub fn sub_guard_token(&self) -> RefAliveToken {
        self.alive_token.clone()
    }

    // ==================== Trait-based access ====================

    /// Get the trait object. Returns `Err(ExpiredReference)` if this is a
    /// borrowed userdata whose token has expired.
    #[inline]
    pub fn get_trait(&self) -> Result<&dyn UserDataTrait, LuaError> {
        if !self.is_alive() {
            return Err(LuaError::ExpiredReference);
        }
        Ok(match &self.data {
            UserdataStorage::Owned(boxed) => boxed.as_ref(),
            UserdataStorage::Borrowed(ptr) => unsafe { &**ptr },
        })
    }

    /// Get the mutable trait object. Returns `Err(ExpiredReference)` if this
    /// is a borrowed userdata whose token has expired.
    #[inline]
    pub fn get_trait_mut(&mut self) -> Result<&mut dyn UserDataTrait, LuaError> {
        if !self.is_alive() {
            return Err(LuaError::ExpiredReference);
        }
        Ok(match &mut self.data {
            UserdataStorage::Owned(boxed) => boxed.as_mut(),
            UserdataStorage::Borrowed(ptr) => unsafe { &mut **ptr },
        })
    }

    /// Get the type name. Returns `"expired_userdata"` if expired.
    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self.get_trait() {
            Ok(t) => t.type_name(),
            Err(_) => "expired_userdata",
        }
    }

    // ==================== Downcast ====================

    /// Downcast to a concrete type. Returns `None` if expired or type mismatch.
    #[inline]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.get_trait().ok()?.as_any().downcast_ref::<T>()
    }

    /// Downcast to a concrete type (mutable).
    #[inline]
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.get_trait_mut().ok()?.as_any_mut().downcast_mut::<T>()
    }

    /// Get raw `&dyn Any` reference.
    pub fn get_data(&self) -> Result<&dyn Any, LuaError> {
        Ok(self.get_trait()?.as_any())
    }

    /// Get raw `&mut dyn Any` reference.
    pub fn get_data_mut(&mut self) -> Result<&mut dyn Any, LuaError> {
        Ok(self.get_trait_mut()?.as_any_mut())
    }

    // ==================== Metatable ====================

    pub fn get_metatable(&self) -> Option<LuaValue> {
        if self.metatable.is_null() {
            None
        } else {
            Some(LuaValue::table(self.metatable))
        }
    }

    pub(crate) fn set_metatable(&mut self, metatable: LuaValue) {
        if let Some(table_ptr) = metatable.as_table_ptr() {
            self.metatable = table_ptr;
        } else if metatable.is_nil() {
            self.metatable = TablePtr::null();
        }
    }
}

impl Drop for LuaUserdata {
    fn drop(&mut self) {
        if let UserdataStorage::Owned(_) = &self.data {
            self.alive_token.set(false);
        }
    }
}

impl fmt::Debug for LuaUserdata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get_trait() {
            Ok(trait_obj) => write!(
                f,
                "Userdata({}@{:p})",
                trait_obj.type_name(),
                trait_obj.as_any() as *const dyn Any
            ),
            Err(_) => write!(f, "Userdata(expired)"),
        }
    }
}
