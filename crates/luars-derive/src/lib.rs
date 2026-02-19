//! Procedural macros for luars userdata system.
//!
//! Provides `#[derive(LuaUserData)]` to auto-generate `UserDataTrait` implementations
//! for Rust structs, exposing public fields and methods to Lua.
//!
//! # Attributes
//!
//! - `#[lua(skip)]` on a field — exclude it from Lua access
//! - `#[lua(readonly)]` on a field — only allow get, not set
//! - `#[lua(name = "...")]` on a field — use a custom Lua-visible name
//!
//! # Auto-detected trait impls
//!
//! Use `#[lua_impl(...)]` on the struct to declare which Rust traits should map to Lua metamethods:
//! - `Display` → `__tostring`
//! - `PartialEq` → `__eq`
//! - `PartialOrd` → `__lt`, `__le`
//!
//! # Example
//!
//! ```ignore
//! use luars_derive::LuaUserData;
//!
//! #[derive(LuaUserData, PartialEq, PartialOrd)]
//! #[lua_impl(Display, PartialEq, PartialOrd)]
//! struct Point {
//!     pub x: f64,
//!     pub y: f64,
//!     #[lua(skip)]
//!     internal_id: u32,
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Meta, parse_macro_input};

/// Derive `UserDataTrait` for a struct, exposing public fields to Lua.
///
/// # Supported field types (auto-converted to/from UdValue)
/// - `i8`, `i16`, `i32`, `i64`, `isize` → `UdValue::Integer`
/// - `u8`, `u16`, `u32`, `u64`, `usize` → `UdValue::Integer`
/// - `f32`, `f64` → `UdValue::Number`
/// - `bool` → `UdValue::Boolean`
/// - `String` → `UdValue::Str`
#[proc_macro_derive(LuaUserData, attributes(lua, lua_impl))]
pub fn derive_lua_userdata(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Parse #[lua_impl(...)] attribute for trait detection
    let trait_impls = parse_lua_impl_attrs(&input);

    // Only works on structs with named fields
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return syn::Error::new_spanned(
                    &input.ident,
                    "LuaUserData can only be derived for structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(
                &input.ident,
                "LuaUserData can only be derived for structs",
            )
            .to_compile_error()
            .into();
        }
    };

    // Collect field info
    let mut field_infos: Vec<FieldInfo> = Vec::new();
    for field in fields.iter() {
        let ident = field.ident.as_ref().unwrap();
        let ty = &field.ty;
        let is_pub = matches!(field.vis, syn::Visibility::Public(_));

        // Parse field attributes
        let mut skip = false;
        let mut readonly = false;
        let mut lua_name: Option<String> = None;

        for attr in &field.attrs {
            if attr.path().is_ident("lua") {
                if let Ok(list) = attr.meta.require_list() {
                    let _ = list.parse_nested_meta(|meta| {
                        if meta.path.is_ident("skip") {
                            skip = true;
                        } else if meta.path.is_ident("readonly") {
                            readonly = true;
                        } else if meta.path.is_ident("name") {
                            if let Ok(value) = meta.value() {
                                if let Ok(lit) = value.parse::<syn::LitStr>() {
                                    lua_name = Some(lit.value());
                                }
                            }
                        }
                        Ok(())
                    });
                }
            }
        }

        if skip || !is_pub {
            continue;
        }

        let name_str = lua_name.unwrap_or_else(|| ident.to_string());
        field_infos.push(FieldInfo {
            ident: ident.clone(),
            ty: ty.clone(),
            lua_name: name_str,
            readonly,
        });
    }

    // Generate get_field match arms
    let get_field_arms = field_infos.iter().map(|f| {
        let ident = &f.ident;
        let lua_name = &f.lua_name;
        let conversion = field_to_udvalue(&f.ty, quote!(self.#ident));
        quote! { #lua_name => Some(#conversion), }
    });

    // Generate set_field match arms (writable fields)
    let set_field_arms = field_infos.iter().filter(|f| !f.readonly).map(|f| {
        let ident = &f.ident;
        let lua_name = &f.lua_name;
        let assign = udvalue_to_field(&f.ty, quote!(self.#ident), lua_name);
        quote! { #lua_name => { #assign } }
    });

    // Generate set_field match arms (readonly fields → error)
    let readonly_set_arms = field_infos.iter().filter(|f| f.readonly).map(|f| {
        let lua_name = &f.lua_name;
        quote! { #lua_name => Some(Err(format!("field '{}' is read-only", #lua_name))), }
    });

    // Generate field_names list
    let field_name_strs: Vec<&String> = field_infos.iter().map(|f| &f.lua_name).collect();

    // Generate metamethod impls based on #[lua_impl(...)]
    let tostring_impl = if trait_impls.contains(&"Display".to_string()) {
        quote! {
            fn lua_tostring(&self) -> Option<String> {
                Some(format!("{}", self))
            }
        }
    } else {
        quote! {}
    };

    let eq_impl = if trait_impls.contains(&"PartialEq".to_string()) {
        quote! {
            fn lua_eq(&self, other: &dyn luars::lua_value::userdata_trait::UserDataTrait) -> Option<bool> {
                other.as_any().downcast_ref::<#name>().map(|o| self == o)
            }
        }
    } else {
        quote! {}
    };

    let ord_impl = if trait_impls.contains(&"PartialOrd".to_string()) {
        quote! {
            fn lua_lt(&self, other: &dyn luars::lua_value::userdata_trait::UserDataTrait) -> Option<bool> {
                other.as_any().downcast_ref::<#name>()
                    .and_then(|o| self.partial_cmp(o))
                    .map(|c| c == std::cmp::Ordering::Less)
            }
            fn lua_le(&self, other: &dyn luars::lua_value::userdata_trait::UserDataTrait) -> Option<bool> {
                other.as_any().downcast_ref::<#name>()
                    .and_then(|o| self.partial_cmp(o))
                    .map(|c| c != std::cmp::Ordering::Greater)
            }
        }
    } else {
        quote! {}
    };

    let type_name_str = name.to_string();

    let expanded = quote! {
        impl luars::lua_value::userdata_trait::UserDataTrait for #name {
            fn type_name(&self) -> &'static str {
                #type_name_str
            }

            fn get_field(&self, key: &str) -> Option<luars::lua_value::userdata_trait::UdValue> {
                match key {
                    #(#get_field_arms)*
                    _ => None,
                }
            }

            fn set_field(&mut self, key: &str, value: luars::lua_value::userdata_trait::UdValue) -> Option<Result<(), String>> {
                match key {
                    #(#set_field_arms)*
                    #(#readonly_set_arms)*
                    _ => None,
                }
            }

            fn field_names(&self) -> &'static [&'static str] {
                &[#(#field_name_strs),*]
            }

            #tostring_impl
            #eq_impl
            #ord_impl

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }
    };

    expanded.into()
}

// ==================== Internal types ====================

struct FieldInfo {
    ident: Ident,
    ty: syn::Type,
    lua_name: String,
    readonly: bool,
}

// ==================== Attribute parsing ====================

/// Parse `#[lua_impl(Display, PartialEq, PartialOrd, ...)]` attributes
fn parse_lua_impl_attrs(input: &DeriveInput) -> Vec<String> {
    let mut impls = Vec::new();
    for attr in &input.attrs {
        if attr.path().is_ident("lua_impl") {
            if let Meta::List(list) = &attr.meta {
                let _ = list.parse_nested_meta(|meta| {
                    if let Some(ident) = meta.path.get_ident() {
                        impls.push(ident.to_string());
                    }
                    Ok(())
                });
            }
        }
    }
    impls
}

// ==================== Code generation helpers ====================

/// Generate code to convert a Rust field value → UdValue
fn field_to_udvalue(
    ty: &syn::Type,
    accessor: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let type_str = normalize_type(ty);

    match type_str.as_str() {
        // Integers → UdValue::Integer
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
            quote! { luars::lua_value::userdata_trait::UdValue::Integer(#accessor as i64) }
        }
        // Floats → UdValue::Number
        "f32" | "f64" => {
            quote! { luars::lua_value::userdata_trait::UdValue::Number(#accessor as f64) }
        }
        // Bool → UdValue::Boolean
        "bool" => {
            quote! { luars::lua_value::userdata_trait::UdValue::Boolean(#accessor) }
        }
        // String → UdValue::Str (cloned)
        "String" => {
            quote! { luars::lua_value::userdata_trait::UdValue::Str(#accessor.clone()) }
        }
        // Fallback: try Into<UdValue>
        _ => {
            quote! { luars::lua_value::userdata_trait::UdValue::from(#accessor.clone()) }
        }
    }
}

/// Generate code to convert UdValue → Rust type and assign to a field
fn udvalue_to_field(
    ty: &syn::Type,
    target: proc_macro2::TokenStream,
    field_name: &str,
) -> proc_macro2::TokenStream {
    let type_str = normalize_type(ty);

    match type_str.as_str() {
        // Integer types
        "i8" | "i16" | "i32" | "i64" | "isize" => {
            quote! {
                match value.to_integer() {
                    Some(i) => { #target = i as #ty; Some(Ok(())) }
                    None => Some(Err(format!("expected integer for field '{}'", #field_name)))
                }
            }
        }
        "u8" | "u16" | "u32" | "u64" | "usize" => {
            quote! {
                match value.to_integer() {
                    Some(i) if i >= 0 => { #target = i as #ty; Some(Ok(())) }
                    Some(_) => Some(Err(format!("expected non-negative integer for field '{}'", #field_name))),
                    None => Some(Err(format!("expected integer for field '{}'", #field_name)))
                }
            }
        }
        // Float types
        "f32" | "f64" => {
            quote! {
                match value.to_number() {
                    Some(n) => { #target = n as #ty; Some(Ok(())) }
                    None => Some(Err(format!("expected number for field '{}'", #field_name)))
                }
            }
        }
        // Bool
        "bool" => {
            quote! {
                {
                    #target = value.to_bool();
                    Some(Ok(()))
                }
            }
        }
        // String
        "String" => {
            quote! {
                match value.to_str() {
                    Some(s) => { #target = s.to_owned(); Some(Ok(())) }
                    None => Some(Err(format!("expected string for field '{}'", #field_name)))
                }
            }
        }
        // Unsupported type
        _ => {
            quote! {
                Some(Err(format!("cannot set field '{}': unsupported type", #field_name)))
            }
        }
    }
}

/// Normalize a syn::Type to a simple string for matching
fn normalize_type(ty: &syn::Type) -> String {
    quote!(#ty).to_string().replace(" ", "")
}
