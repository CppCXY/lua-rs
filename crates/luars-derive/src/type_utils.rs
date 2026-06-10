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

/// Check whether a normalized type is `RefAliveToken`.
pub fn is_ref_alive_token_type(normalized: &str) -> bool {
    normalized == "RefAliveToken" || normalized.ends_with("::RefAliveToken")
}

/// Check whether a normalized type string is a "primitive" that should
/// use value semantics (copy/clone) rather than sub-reference.
///
/// Non-primitive types (userdata types like `Vec3`, `Player`, etc.)
/// should return sub-references.
pub fn is_primitive_type(normalized: &str) -> bool {
    matches!(
        normalized,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "String"
    )
}

/// Check whether a normalized type is a string-like type.
///
/// `str` and `String` should never be treated as userdata — they
/// represent Lua strings and should go through `IntoLua`/`FromLua`
/// conversions instead of sub-reference wrapping.
pub fn is_string_like_type(normalized: &str) -> bool {
    matches!(normalized, "str" | "String")
}

/// Check whether a normalized type should use value semantics (copy/clone
/// via `UdValue::from`) rather than userdata sub-reference wrapping.
///
/// Returns `true` for primitive types, string-like types, and their
/// combinations with references / Option / Result wrappers.
/// Returns `false` for types that should be treated as userdata.
pub fn is_value_type(normalized: &str) -> bool {
    if is_primitive_type(normalized) || is_string_like_type(normalized) {
        return true;
    }
    if let Some((inner, _)) = strip_reference(normalized) {
        return is_value_type(inner);
    }
    if let Some((inner, _)) = unwrap_outer_type(normalized) {
        return is_value_type(inner);
    }
    false
}

/// Strip leading `&` or `&mut` and any lifetime from a reference type string.
///
/// `"&'aCounter"` → `("Counter", "&")`, `"&mutCounter"` → `("Counter", "&mut")`.
/// Returns `None` if the type is not a reference.
pub fn strip_reference(normalized: &str) -> Option<(&str, &str)> {
    if let Some(rest) = normalized.strip_prefix("&mut") {
        let inner = strip_lifetime(rest);
        Some((inner, "&mut"))
    } else if let Some(rest) = normalized.strip_prefix('&') {
        let inner = strip_lifetime(rest);
        Some((inner, "&"))
    } else {
        None
    }
}

/// Strip lifetime prefix like `'a` from a string.
fn strip_lifetime(s: &str) -> &str {
    if s.starts_with('\'') {
        s.trim_start_matches(|c: char| c == '\'' || c.is_alphanumeric())
    } else {
        s
    }
}

/// Unwrap a single layer of `Option<...>` or `Result<..., E>` and return
/// the inner type + the wrapper kind.
#[derive(Debug, PartialEq)]
pub enum WrapperKind {
    /// `Option<T>`
    Option,
    /// `Result<T, E>`
    Result,
}

/// If `normalized` is `Option<Inner>` or `Result<Inner, Error>`,
/// return `(Inner, WrapperKind)`. Otherwise `None`.
pub fn unwrap_outer_type(normalized: &str) -> Option<(&str, WrapperKind)> {
    if let Some(inner) = normalized
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return Some((inner, WrapperKind::Option));
    }
    if let Some(rest) = normalized.strip_prefix("Result<") {
        // Result<Inner, Error> — split on first comma at depth 0
        if let Some(inner) = split_at_top_level_comma(rest) {
            return Some((inner, WrapperKind::Result));
        }
    }
    None
}

/// Split `T,E` at the top-level comma (respects nested `<>`).
/// Returns the part before the comma.
fn split_at_top_level_comma(s: &str) -> Option<&str> {
    let mut depth = 0u32;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                if depth == 0 {
                    return None; // closing > without opening < — not ours
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                let inner = &s[..i];
                // Ensure there's something after the comma (the error type)
                if i + 1 < s.len() {
                    return Some(inner);
                }
                return None;
            }
            _ => {}
        }
    }
    None
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
            quote! { luars::UdValue::Integer(#accessor as i64) }
        }
        // Floats → UdValue::Number
        "f32" | "f64" => {
            quote! { luars::UdValue::Number(#accessor as f64) }
        }
        // Bool → UdValue::Boolean
        "bool" => {
            quote! { luars::UdValue::Boolean(#accessor) }
        }
        // String → UdValue::Str (cloned)
        "String" => {
            quote! { luars::UdValue::Str(#accessor.clone()) }
        }
        // Fallback: try Into<UdValue>
        _ => {
            quote! { luars::UdValue::from(#accessor.clone()) }
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

/// Generate code to convert a `&T` reference to `UdValue`.
///
/// Used for iterating over collection elements (e.g., `Vec<T>`).
/// The `accessor` expression should evaluate to a `&T`.
pub fn ref_to_udvalue(
    ty: &syn::Type,
    accessor: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let type_str = normalize_type(ty);

    match type_str.as_str() {
        // Integers → dereference then cast
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
            quote! { luars::UdValue::Integer(*#accessor as i64) }
        }
        // Floats
        "f32" | "f64" => {
            quote! { luars::UdValue::Number(*#accessor as f64) }
        }
        // Bool
        "bool" => {
            quote! { luars::UdValue::Boolean(*#accessor) }
        }
        // String → clone the reference
        "String" => {
            quote! { luars::UdValue::Str(#accessor.clone()) }
        }
        // Fallback: clone and convert via From/Into
        _ => {
            quote! { luars::UdValue::from(#accessor.clone()) }
        }
    }
}
