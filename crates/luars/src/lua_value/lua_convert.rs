//! `FromLua` / `IntoLua` — bidirectional conversion between Rust types and `LuaValue`.
//!
//! These traits allow Lua function arguments and return values to be expressed
//! with native Rust types instead of manually calling `get_arg` / `push_value`.
//!
//! # Built-in impls
//! - `()`, `bool`, `i8`..`i64`, `u8`..`u64`, `f32`, `f64`
//! - `String`, `&str` (via intermediate `String`)
//! - `Option<T>` where `T: FromLua` / `T: IntoLua`
//! - `LuaValue` (identity — zero-cost passthrough)
//!
//! # Usage in derive macros
//! The `#[lua_methods]` macro generates calls to `FromLua::from_lua` for each
//! parameter and `IntoLua::into_lua` for the return value, keeping the codegen
//! type-agnostic and user-extensible.
//!
//! # User extensibility
//! Users can implement `FromLua` / `IntoLua` for their own types:
//! ```ignore
//! impl FromLua for MyVec3 {
//!     fn from_lua(value: LuaValue, state: &LuaState) -> Result<Self, String> {
//!         // extract from userdata or table
//!     }
//! }
//! ```

use crate::lua_value::LuaValue;
use crate::lua_vm::LuaState;

/// Convert a `LuaValue` into a Rust type.
///
/// Implementors define how a Lua value is converted to `Self`.
/// Return `Err(message)` for type mismatches.
pub trait FromLua: Sized {
    /// Convert a `LuaValue` to `Self`.
    ///
    /// `state` is provided for operations that need GC access (e.g. string interning).
    fn from_lua(value: LuaValue, state: &LuaState) -> Result<Self, String>;
}

/// Convert a Rust type into a `LuaValue` and push it.
///
/// Implementors define how `self` becomes one or more Lua values on the stack.
/// Returns the number of values pushed (typically 1, or 0 for `()`).
pub trait IntoLua {
    /// Push this value onto the Lua stack.
    ///
    /// Returns the number of Lua values pushed.
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String>;
}

// ==================== Identity: LuaValue ====================

impl FromLua for LuaValue {
    #[inline]
    fn from_lua(value: LuaValue, _state: &LuaState) -> Result<Self, String> {
        Ok(value)
    }
}

impl IntoLua for LuaValue {
    #[inline]
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        state.push_value(self).map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

// ==================== Unit ====================

impl FromLua for () {
    #[inline]
    fn from_lua(_value: LuaValue, _state: &LuaState) -> Result<Self, String> {
        Ok(())
    }
}

impl IntoLua for () {
    #[inline]
    fn into_lua(self, _state: &mut LuaState) -> Result<usize, String> {
        Ok(0)
    }
}

// ==================== Boolean ====================

impl FromLua for bool {
    #[inline]
    fn from_lua(value: LuaValue, _state: &LuaState) -> Result<Self, String> {
        // Follow Lua truthiness: nil and false → false, everything else → true
        Ok(value.as_boolean().unwrap_or(!value.is_nil()))
    }
}

impl IntoLua for bool {
    #[inline]
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        state
            .push_value(LuaValue::boolean(self))
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

// ==================== Integer types ====================

macro_rules! impl_from_lua_int {
    ($($ty:ty),*) => {
        $(
            impl FromLua for $ty {
                #[inline]
                fn from_lua(value: LuaValue, _state: &LuaState) -> Result<Self, String> {
                    if let Some(i) = value.as_integer() {
                        Ok(i as $ty)
                    } else if let Some(f) = value.as_float() {
                        Ok(f as $ty)
                    } else {
                        Err(format!("expected integer, got {}", value.type_name()))
                    }
                }
            }

            impl IntoLua for $ty {
                #[inline]
                fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
                    state
                        .push_value(LuaValue::integer(self as i64))
                        .map_err(|e| format!("{:?}", e))?;
                    Ok(1)
                }
            }
        )*
    };
}

impl_from_lua_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

// ==================== Float types ====================

macro_rules! impl_from_lua_float {
    ($($ty:ty),*) => {
        $(
            impl FromLua for $ty {
                #[inline]
                fn from_lua(value: LuaValue, _state: &LuaState) -> Result<Self, String> {
                    if let Some(n) = value.as_number() {
                        Ok(n as $ty)
                    } else if let Some(i) = value.as_integer() {
                        Ok(i as $ty)
                    } else {
                        Err(format!("expected number, got {}", value.type_name()))
                    }
                }
            }

            impl IntoLua for $ty {
                #[inline]
                fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
                    state
                        .push_value(LuaValue::float(self as f64))
                        .map_err(|e| format!("{:?}", e))?;
                    Ok(1)
                }
            }
        )*
    };
}

impl_from_lua_float!(f32, f64);

// ==================== String ====================

impl FromLua for String {
    #[inline]
    fn from_lua(value: LuaValue, _state: &LuaState) -> Result<Self, String> {
        if let Some(s) = value.as_str() {
            Ok(s.to_owned())
        } else if let Some(i) = value.as_integer() {
            // Lua coerces numbers to strings
            Ok(format!("{}", i))
        } else if let Some(f) = value.as_float() {
            Ok(format!("{}", f))
        } else {
            Err(format!("expected string, got {}", value.type_name()))
        }
    }
}

impl IntoLua for String {
    #[inline]
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        let s = state.create_string(&self).map_err(|e| format!("{:?}", e))?;
        state.push_value(s).map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl IntoLua for &str {
    #[inline]
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        let s = state.create_string(self).map_err(|e| format!("{:?}", e))?;
        state.push_value(s).map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

// ==================== Option<T> ====================

impl<T: FromLua> FromLua for Option<T> {
    #[inline]
    fn from_lua(value: LuaValue, state: &LuaState) -> Result<Self, String> {
        if value.is_nil() {
            Ok(None)
        } else {
            T::from_lua(value, state).map(Some)
        }
    }
}

impl<T: IntoLua> IntoLua for Option<T> {
    #[inline]
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        match self {
            Some(v) => v.into_lua(state),
            None => {
                state
                    .push_value(LuaValue::nil())
                    .map_err(|e| format!("{:?}", e))?;
                Ok(1)
            }
        }
    }
}

// ==================== Result<T, E> ====================

impl<T: IntoLua, E: std::fmt::Display> IntoLua for Result<T, E> {
    #[inline]
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        match self {
            Ok(v) => v.into_lua(state),
            Err(e) => Err(format!("{}", e)),
        }
    }
}

// ==================== Vec<T> (push as multiple returns) ====================

impl<T: IntoLua> IntoLua for Vec<T> {
    #[inline]
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        let count = self.len();
        for item in self {
            item.into_lua(state)?;
        }
        Ok(count)
    }
}
