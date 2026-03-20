//! Procedural macros for luars userdata system.
//!
//! # Macros provided
//!
//! - `#[derive(LuaUserData)]` — auto-generate `UserDataTrait` for structs and enums
//!   (field access via `get_field`/`set_field`, metamethods via `#[lua_impl(...)]`,
//!   and `IntoLua` for owned Rust → Lua userdata conversion)
//!
//! - `#[lua_methods]` — attribute macro on impl blocks, generates static C wrapper
//!   functions for each `pub fn`, accessible from Lua via `obj:method(...)` calls
//!
//! # Architecture
//!
//! - `derive_userdata.rs` — `#[derive(LuaUserData)]` implementation
//! - `lua_methods.rs` — `#[lua_methods]` implementation
//! - `type_utils.rs` — shared type conversion helpers (Rust ↔ UdValue ↔ LuaValue)

mod derive_userdata;
mod lua_methods;
mod type_utils;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Derive `UserDataTrait` for a struct or enum.
///
/// # Supported field types (auto-converted to/from UdValue)
/// - `i8`..`i64`, `isize` → `UdValue::Integer`
/// - `u8`..`u64`, `usize` → `UdValue::Integer`
/// - `f32`, `f64` → `UdValue::Number`
/// - `bool` → `UdValue::Boolean`
/// - `String` → `UdValue::Str`
///
/// # Field attributes
/// - `#[lua(skip)]` — exclude from Lua
/// - `#[lua(readonly)]` — get only, no set
/// - `#[lua(name = "...")]` — custom Lua name
///
/// # Struct attributes
/// - `#[lua_impl(Display, PartialEq, PartialOrd)]` — metamethods from Rust traits
///
/// # Enum behavior
/// - C-like enums also implement `LuaEnum`, so they can be exported with `register_enum::<T>()`
/// - Enums with payloads are treated as fieldless userdata and can expose methods via `#[lua_methods]`
///
/// # Conversion behavior
/// - `IntoLua` is auto-implemented, so derived userdata values can be passed directly into typed APIs
/// - Owned `FromLua` is intentionally not auto-implemented, because userdata lives in Lua GC storage and
///   implicit extraction by value would blur ownership semantics
///
/// # Example
/// ```ignore
/// #[derive(LuaUserData, PartialEq, PartialOrd)]
/// #[lua_impl(Display, PartialEq, PartialOrd)]
/// struct Point {
///     pub x: f64,
///     pub y: f64,
///     #[lua(skip)]
///     internal_id: u32,
/// }
/// ```
#[proc_macro_derive(LuaUserData, attributes(lua, lua_impl))]
pub fn derive_lua_userdata(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    derive_userdata::derive_lua_userdata_impl(input)
}

/// Attribute macro on impl blocks — exposes public methods to Lua.
///
/// For each `pub fn` with a `&self` or `&mut self` receiver, generates:
/// 1. A static `fn(l: &mut LuaState) -> LuaResult<usize>` wrapper
/// 2. Automatic parameter extraction from Lua stack
/// 3. Automatic return value conversion to Lua
///
/// Methods are accessible from Lua via `obj:method(args)` syntax.
///
/// # Example
/// ```ignore
/// #[lua_methods]
/// impl Point {
///     pub fn distance(&self) -> f64 {
///         (self.x * self.x + self.y * self.y).sqrt()
///     }
///     pub fn translate(&mut self, dx: f64, dy: f64) {
///         self.x += dx;
///         self.y += dy;
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn lua_methods(_attr: TokenStream, input: TokenStream) -> TokenStream {
    lua_methods::lua_methods_impl(input)
}
