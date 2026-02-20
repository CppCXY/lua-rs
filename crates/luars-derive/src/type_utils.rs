//! Shared type conversion utilities for luars-derive.
//!
//! Handles mapping between Rust types and `UdValue` variants,
//! used by both `#[derive(LuaUserData)]` (field access) and
//! `#[lua_methods]` (parameter/return value conversion).

use quote::quote;

/// Normalize a `syn::Type` to a simple string for matching.
///
/// Strips whitespace so `Option < i64 >` becomes `Option<i64>`.
pub fn normalize_type(ty: &syn::Type) -> String {
    quote!(#ty).to_string().replace(" ", "")
}

/// Generate code to convert a Rust field value → `UdValue`.
///
/// Used by the derive macro's `get_field` implementation.
pub fn field_to_udvalue(
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

/// Generate code to convert `UdValue` → Rust type and assign to a field.
///
/// Used by the derive macro's `set_field` implementation.
pub fn udvalue_to_field(
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
