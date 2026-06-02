use std::{any::Any, fmt};

use crate::{LuaValue, RefAliveToken, UserDataTrait, gc::TablePtr};

/// Userdata storage — either owns the data or borrows it via raw pointer.
///
/// When `Owned`, the data lives as long as this userdata (GC-managed).
/// When `Borrowed`, the data has an external lifetime (caller's responsibility).
pub enum UserdataStorage {
    Owned(Box<dyn UserDataTrait>),
    Borrowed(*mut dyn UserDataTrait),
}

/// Userdata - arbitrary Rust data with optional metatable.
///
/// Storage is either [`UserdataStorage::Owned`] (GC-managed) or
/// [`UserdataStorage::Borrowed`] (raw pointer — replaces `RefUserData`).
///
/// Every `LuaUserdata` carries a `sub_guard` token that can be cloned
/// to create [`SubRef`](crate::SubRef) sub-references. When an `Owned`
/// userdata is dropped, the guard flips to `false`, invalidating all
/// sub-references.
pub struct LuaUserdata {
    data: UserdataStorage,
    metatable: TablePtr,
    /// Liveness token — cloned by sub-references. Flipped to false
    /// when this userdata is dropped (only for Owned storage).
    alive_token: RefAliveToken,
}

impl LuaUserdata {
    /// Create a new **owned** userdata. A sub-ref guard is automatically created.
    pub fn new<T: UserDataTrait>(data: T) -> Self {
        LuaUserdata {
            data: UserdataStorage::Owned(Box::new(data)),
            metatable: TablePtr::null(),
            alive_token: RefAliveToken::default(),
        }
    }

    /// Create an owned userdata from an already-boxed trait object.
    ///
    /// Used by the VM to convert `UdValue::UserdataOwned` results from
    /// arithmetic trait methods into GC-managed userdata.
    pub fn from_boxed(data: Box<dyn UserDataTrait>) -> Self {
        LuaUserdata {
            data: UserdataStorage::Owned(data),
            metatable: TablePtr::null(),
            alive_token: RefAliveToken::default(),
        }
    }

    /// Create a **borrowed** userdata from a mutable reference.
    ///
    /// The resulting userdata forwards all field/method/metamethod access through
    /// a raw pointer — zero overhead, no ownership transfer.
    ///
    /// Sub-references **cannot** be created from borrowed userdata (the guard is
    /// permanently dead for borrowed storage).
    ///
    /// # Safety
    /// The referenced object **must** outlive all Lua accesses to this userdata.
    /// Accessing the userdata after the Rust object is dropped is **undefined behavior**.
    #[inline]
    pub fn from_ref<T: UserDataTrait>(reference: &mut T, token: RefAliveToken) -> Self {
        LuaUserdata {
            data: UserdataStorage::Borrowed(reference as *mut T as *mut dyn UserDataTrait),
            metatable: TablePtr::null(),
            alive_token: token, // borrowed
        }
    }

    #[inline]
    pub fn from_ptr<T: UserDataTrait + 'static>(ptr: *const T, token: RefAliveToken) -> Self {
        LuaUserdata {
            data: UserdataStorage::Borrowed(ptr as *mut T as *mut dyn UserDataTrait),
            metatable: TablePtr::null(),
            alive_token: token, // borrowed
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

    /// Get the trait object for direct field/method/metamethod dispatch.
    #[inline]
    pub fn get_trait(&self) -> &dyn UserDataTrait {
        match &self.data {
            UserdataStorage::Owned(boxed) => boxed.as_ref(),
            UserdataStorage::Borrowed(ptr) => unsafe { &**ptr },
        }
    }

    /// Get the mutable trait object.
    #[inline]
    pub fn get_trait_mut(&mut self) -> &mut dyn UserDataTrait {
        match &mut self.data {
            UserdataStorage::Owned(boxed) => boxed.as_mut(),
            UserdataStorage::Borrowed(ptr) => unsafe { &mut **ptr },
        }
    }

    /// Get the type name from the trait.
    #[inline]
    pub fn type_name(&self) -> &'static str {
        self.get_trait().type_name()
    }

    // ==================== Sub-ref guard ====================

    /// Clone a sub-ref token tied to this userdata's lifetime.
    ///
    /// Returns `None` for borrowed userdata (no sub-ref support).
    #[inline]
    pub fn sub_guard_token(&self) -> RefAliveToken {
        self.alive_token.clone()
    }

    /// Returns `true` if this is an owned userdata (not borrowed).
    #[inline]
    pub fn is_owned(&self) -> bool {
        matches!(self.data, UserdataStorage::Owned(_))
    }

    pub fn is_alive(&self) -> bool {
        match self.data {
            UserdataStorage::Owned(_) => true,
            UserdataStorage::Borrowed(_) => self.alive_token.is_alive(),
        }
    }

    // ==================== Backward-compatible downcast access ====================

    /// Downcast to a concrete type (immutable).
    #[inline]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.get_trait().as_any().downcast_ref::<T>()
    }

    /// Downcast to a concrete type (mutable).
    #[inline]
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.get_trait_mut().as_any_mut().downcast_mut::<T>()
    }

    /// Get raw `&dyn Any` reference (backward compatibility).
    pub fn get_data(&self) -> &dyn Any {
        self.get_trait().as_any()
    }

    /// Get raw `&mut dyn Any` reference (backward compatibility).
    pub fn get_data_mut(&mut self) -> &mut dyn Any {
        self.get_trait_mut().as_any_mut()
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
        } else {
            debug_assert!(
                false,
                "Attempted to set userdata metatable to non-table, non-nil value"
            );
        }
    }
}

impl Drop for LuaUserdata {
    fn drop(&mut self) {
        if let UserdataStorage::Owned(_) = &self.data {
            // Flip the guard first — the Box is dropped afterwards (fields
            // are dropped in declaration order: data, metatable, sub_guard).
            self.alive_token.set(false);
        }
    }
}

impl fmt::Debug for LuaUserdata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Userdata({}@{:p})",
            self.get_trait().type_name(),
            self.get_trait().as_any() as *const dyn Any
        )
    }
}
