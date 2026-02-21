//! Builder-pattern wrapper for creating userdata from third-party types.
//!
//! [`UserDataBuilder`] lets you expose fields, methods, and metamethods for
//! types you don't control (no derive macro available).
//!
//! # Example
//!
//! ```ignore
//! use std::net::SocketAddr;
//! use luars::UserDataBuilder;
//!
//! let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
//! let ud = UserDataBuilder::new(addr)
//!     .add_field_getter("ip", |a| UdValue::Str(a.ip().to_string()))
//!     .add_field_getter("port", |a| UdValue::Integer(a.port() as i64))
//!     .set_tostring(|a| a.to_string())
//!     .build(&mut vm)?;
//! vm.set_global("server_addr", ud)?;
//! ```

use std::any::Any;
use std::collections::HashMap;

use super::LuaUserdata;
use super::userdata_trait::{UdValue, UserDataTrait};

/// Builder for creating userdata from arbitrary types.
///
/// Collects field getters, field setters, and metamethods, then produces a
/// [`LuaValue`](crate::LuaValue) userdata when [`build`](Self::build) is called.
pub struct UserDataBuilder<T: 'static> {
    value: T,
    type_name: &'static str,
    field_getters: HashMap<String, Box<dyn Fn(&T) -> UdValue>>,
    field_setters: HashMap<String, Box<dyn Fn(&mut T, UdValue) -> Result<(), String>>>,
    tostring_fn: Option<Box<dyn Fn(&T) -> String>>,
}

impl<T: 'static> UserDataBuilder<T> {
    /// Start building userdata wrapping `value`.
    ///
    /// The default type name is `std::any::type_name::<T>()`.
    pub fn new(value: T) -> Self {
        UserDataBuilder {
            value,
            type_name: std::any::type_name::<T>(),
            field_getters: HashMap::new(),
            field_setters: HashMap::new(),
            tostring_fn: None,
        }
    }

    /// Override the type name shown in Lua `type()` calls and error messages.
    pub fn set_type_name(mut self, name: &'static str) -> Self {
        self.type_name = name;
        self
    }

    /// Register a read-only field.
    ///
    /// ```ignore
    /// .add_field_getter("ip", |addr| UdValue::Str(addr.ip().to_string()))
    /// ```
    pub fn add_field_getter<F>(mut self, name: &str, f: F) -> Self
    where
        F: Fn(&T) -> UdValue + 'static,
    {
        self.field_getters.insert(name.to_owned(), Box::new(f));
        self
    }

    /// Register a writable field.
    ///
    /// The setter receives the new `UdValue` and should return `Ok(())` on
    /// success or `Err(msg)` if the value is invalid.
    pub fn add_field_setter<F>(mut self, name: &str, f: F) -> Self
    where
        F: Fn(&mut T, UdValue) -> Result<(), String> + 'static,
    {
        self.field_setters.insert(name.to_owned(), Box::new(f));
        self
    }

    /// Add both a getter and a setter for the same field name.
    pub fn add_field<G, S>(self, name: &str, getter: G, setter: S) -> Self
    where
        G: Fn(&T) -> UdValue + 'static,
        S: Fn(&mut T, UdValue) -> Result<(), String> + 'static,
    {
        self.add_field_getter(name, getter)
            .add_field_setter(name, setter)
    }

    /// Set the `__tostring` metamethod.
    pub fn set_tostring<F>(mut self, f: F) -> Self
    where
        F: Fn(&T) -> String + 'static,
    {
        self.tostring_fn = Some(Box::new(f));
        self
    }

    /// Consume the builder and create a `LuaValue` userdata in the VM.
    pub fn build(self, vm: &mut crate::lua_vm::LuaVM) -> crate::LuaResult<crate::LuaValue> {
        let configured = ConfiguredUserData {
            value: self.value,
            type_name: self.type_name,
            field_getters: self.field_getters,
            field_setters: self.field_setters,
            tostring_fn: self.tostring_fn,
        };
        let ud = LuaUserdata::new(configured);
        vm.create_userdata(ud)
    }
}

// ---- internal trait object wrapper ------------------------------------------

/// Holds the value together with the user-supplied accessor closures.
struct ConfiguredUserData<T: 'static> {
    value: T,
    type_name: &'static str,
    field_getters: HashMap<String, Box<dyn Fn(&T) -> UdValue>>,
    field_setters: HashMap<String, Box<dyn Fn(&mut T, UdValue) -> Result<(), String>>>,
    tostring_fn: Option<Box<dyn Fn(&T) -> String>>,
}

impl<T: 'static> UserDataTrait for ConfiguredUserData<T> {
    fn type_name(&self) -> &'static str {
        self.type_name
    }

    fn get_field(&self, key: &str) -> Option<UdValue> {
        self.field_getters.get(key).map(|f| f(&self.value))
    }

    fn set_field(&mut self, key: &str, value: UdValue) -> Option<Result<(), String>> {
        if let Some(setter) = self.field_setters.get(key) {
            Some(setter(&mut self.value, value))
        } else {
            None
        }
    }

    fn lua_tostring(&self) -> Option<String> {
        self.tostring_fn.as_ref().map(|f| f(&self.value))
    }

    fn field_names(&self) -> &'static [&'static str] {
        // Dynamic fields — we can't return a static slice of the HashMap keys,
        // so return empty. The getters still work via get_field dispatch.
        &[]
    }

    fn as_any(&self) -> &dyn Any {
        &self.value
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.value
    }
}
