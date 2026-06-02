use std::{any::Any, fmt};

use crate::{LuaValue, RefAliveToken, UdValue, UserDataTrait, gc::TablePtr};
use crate::lua_vm::CFunction;

/// Userdata storage — either owns the data or borrows it via raw pointer.
///
/// When `Owned`, the data lives as long as this userdata (GC-managed).
/// When `Borrowed`, the data has an external lifetime (caller's responsibility).
pub enum UserdataStorage {
    Owned(Box<dyn UserDataTrait>),
    Borrowed(*mut dyn UserDataTrait),
}

/// Dead sentinel returned when accessing expired borrowed userdata.
/// Viable as a safety net — all paths return nil/None/error.
struct DeadUserdata;

impl UserDataTrait for DeadUserdata {
    fn type_name(&self) -> &'static str { "expired_userdata" }
    fn get_field(&self, _key: &str) -> Option<UdValue> { None }
    fn set_field(&mut self, _key: &str, _value: UdValue) -> Option<Result<(), String>> {
        Some(Err("cannot modify expired sub-reference".into()))
    }
    fn lua_tostring(&self) -> Option<String> { None }
    fn lua_eq(&self, _other: &dyn UserDataTrait) -> Option<bool> { None }
    fn lua_lt(&self, _other: &dyn UserDataTrait) -> Option<bool> { None }
    fn lua_le(&self, _other: &dyn UserDataTrait) -> Option<bool> { None }
    fn lua_len(&self) -> Option<UdValue> { None }
    fn lua_unm(&self) -> Option<UdValue> { None }
    fn lua_bnot(&self) -> Option<UdValue> { None }
    fn lua_add(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_sub(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_mul(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_div(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_mod(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_pow(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_idiv(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_band(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_bor(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_bxor(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_shl(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_shr(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_concat(&self, _other: &UdValue) -> Option<UdValue> { None }
    fn lua_call(&self) -> Option<CFunction> { None }
    fn lua_next(&self, _control: &UdValue) -> Option<(UdValue, UdValue)> { None }
    fn field_names(&self) -> &'static [&'static str] { &[] }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

/// Userdata - arbitrary Rust data with optional metatable.
///
/// Storage is either [`UserdataStorage::Owned`] (GC-managed) or
/// [`UserdataStorage::Borrowed`] (raw pointer, tracked by [`RefAliveToken`]).
///
/// When an `Owned` userdata is dropped, the token flips to `false`,
/// invalidating all borrowed sub-references that share this token.
pub struct LuaUserdata {
    data: UserdataStorage,
    metatable: TablePtr,
    alive_token: RefAliveToken,
}

impl LuaUserdata {
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
    ///
    /// Accesses check `token.is_alive()` before dereferencing the pointer.
    ///
    /// # Safety
    /// The referenced object must outlive the token.
    #[inline]
    pub fn from_ref<T: UserDataTrait>(reference: &mut T, token: RefAliveToken) -> Self {
        LuaUserdata {
            data: UserdataStorage::Borrowed(reference as *mut T as *mut dyn UserDataTrait),
            metatable: TablePtr::null(),
            alive_token: token,
        }
    }

    /// Create a borrowed userdata from a const pointer + liveness token.
    #[inline]
    pub fn from_ptr<T: UserDataTrait + 'static>(ptr: *const T, token: RefAliveToken) -> Self {
        LuaUserdata {
            data: UserdataStorage::Borrowed(ptr as *mut T as *mut dyn UserDataTrait),
            metatable: TablePtr::null(),
            alive_token: token,
        }
    }

    /// Create a borrowed userdata from a raw trait object pointer.
    #[inline]
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

    // ==================== Trait-based access ====================

    /// Get the trait object. For expired borrowed userdata, returns a dead
    /// sentinel that yields nil/None for all operations.
    ///
    /// Callers should prefer `check_alive_or_error()` for explicit error messages
    /// at key entry points.
    #[inline]
    pub fn get_trait(&self) -> &dyn UserDataTrait {
        match &self.data {
            UserdataStorage::Owned(boxed) => boxed.as_ref(),
            UserdataStorage::Borrowed(ptr) => {
                if self.alive_token.is_alive() {
                    unsafe { &**ptr }
                } else {
                    static DEAD: DeadUserdata = DeadUserdata;
                    &DEAD
                }
            }
        }
    }

    /// Get the mutable trait object. For expired borrowed userdata, returns a
    /// dead sentinel.
    #[inline]
    pub fn get_trait_mut(&mut self) -> &mut dyn UserDataTrait {
        match &mut self.data {
            UserdataStorage::Owned(boxed) => boxed.as_mut(),
            UserdataStorage::Borrowed(ptr) => {
                if self.alive_token.is_alive() {
                    unsafe { &mut **ptr }
                } else {
                    // Leak is acceptable — only on error path, ZST
                    Box::leak(Box::new(DeadUserdata))
                }
            }
        }
    }

    /// Returns `Ok(())` if this userdata is safe to access, or an error
    /// with a descriptive message if the backing data has expired.
    #[inline]
    pub fn check_alive_or_error(&self) -> Result<(), String> {
        if !self.is_alive() {
            Err("attempt to use an expired reference — the owning userdata has been garbage collected".into())
        } else {
            Ok(())
        }
    }

    /// Get the type name from the trait.
    #[inline]
    pub fn type_name(&self) -> &'static str {
        self.get_trait().type_name()
    }

    // ==================== Token access ====================

    /// Clone the liveness token (for creating sub-references).
    #[inline]
    pub fn sub_guard_token(&self) -> RefAliveToken {
        self.alive_token.clone()
    }

    /// Returns `true` if this is an owned userdata.
    #[inline]
    pub fn is_owned(&self) -> bool {
        matches!(self.data, UserdataStorage::Owned(_))
    }

    /// Returns `true` if the backing data is still alive.
    #[inline]
    pub fn is_alive(&self) -> bool {
        match &self.data {
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
