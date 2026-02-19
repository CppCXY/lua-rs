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

/// Generate code to extract a Lua argument from `LuaState` and convert to a Rust type.
///
/// `arg_index` is 1-based (Lua convention). For method calls, `self` is arg 1,
/// so user parameters start at arg 2.
///
/// Returns code that evaluates to the Rust value (or errors via `return Err(...)`).
pub fn lua_arg_to_rust(
    ty: &syn::Type,
    arg_index: usize,
    param_name: &str,
) -> proc_macro2::TokenStream {
    let type_str = normalize_type(ty);

    // Check for Option<T> wrapper
    if let Some(inner) = strip_option(&type_str) {
        let inner_extract = lua_arg_extract_inner(&inner, arg_index, param_name, true);
        return inner_extract;
    }

    lua_arg_extract_inner(&type_str, arg_index, param_name, false)
}

/// Generate code to convert a Rust return value and push it onto the Lua stack.
///
/// Returns code that evaluates to a `LuaResult<usize>` (number of return values).
pub fn rust_return_to_lua(ty: &syn::Type) -> proc_macro2::TokenStream {
    let type_str = normalize_type(ty);

    // Result<T, E> → unwrap or error
    if let Some(ok_type) = strip_result(&type_str) {
        let push_ok = rust_value_push_code(&ok_type, quote!(__ok_val));
        return quote! {
            match __result {
                Ok(__ok_val) => {
                    #push_ok
                }
                Err(__err) => {
                    return Err(__l.error(format!("{}", __err)));
                }
            }
        };
    }

    // Option<T> → push value or nil
    if let Some(inner) = strip_option(&type_str) {
        let push_some = rust_value_push_code(&inner, quote!(__some_val));
        return quote! {
            match __result {
                Some(__some_val) => {
                    #push_some
                }
                None => {
                    __l.push_value(luars::LuaValue::nil())?;
                    Ok(1)
                }
            }
        };
    }

    // Plain type
    rust_value_push_code(&type_str, quote!(__result))
}

// ==================== Internal helpers ====================

/// Extract inner type from `Option<T>` string, returns None if not Option.
fn strip_option(type_str: &str) -> Option<String> {
    if type_str.starts_with("Option<") && type_str.ends_with('>') {
        Some(type_str[7..type_str.len() - 1].to_string())
    } else {
        None
    }
}

/// Extract Ok type from `Result<T,E>` string, returns None if not Result.
fn strip_result(type_str: &str) -> Option<String> {
    if type_str.starts_with("Result<") && type_str.ends_with('>') {
        let inner = &type_str[7..type_str.len() - 1];
        // Find the comma separating T and E (handle nested generics)
        let mut depth = 0;
        for (i, c) in inner.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth == 0 => {
                    return Some(inner[..i].to_string());
                }
                _ => {}
            }
        }
        None
    } else {
        None
    }
}

/// Generate extraction code for a single arg (inner type, not wrapped in Option).
fn lua_arg_extract_inner(
    type_str: &str,
    arg_index: usize,
    param_name: &str,
    is_optional: bool,
) -> proc_macro2::TokenStream {
    if is_optional {
        // Optional parameter: nil → None, missing → None
        match type_str {
            "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
                quote! {
                    match __l.get_arg(#arg_index) {
                        Some(v) if !v.is_nil() => match v.as_integer() {
                            Some(i) => Some(i),
                            None => match v.as_float() {
                                Some(f) => Some(f as i64),
                                None => return Err(__l.error(
                                    format!("bad argument #{} ('{}': expected integer, got {:?})", #arg_index, #param_name, v)
                                )),
                            }
                        },
                        _ => None,
                    }
                }
            }
            "f32" | "f64" => {
                quote! {
                    match __l.get_arg(#arg_index) {
                        Some(v) if !v.is_nil() => match v.as_number() {
                            Some(n) => Some(n),
                            None => return Err(__l.error(
                                format!("bad argument #{} ('{}': expected number)", #arg_index, #param_name)
                            )),
                        },
                        _ => None,
                    }
                }
            }
            "bool" => {
                quote! {
                    match __l.get_arg(#arg_index) {
                        Some(v) if !v.is_nil() => Some(v.as_boolean().unwrap_or(true)),
                        _ => None,
                    }
                }
            }
            "String" => {
                quote! {
                    match __l.get_arg(#arg_index) {
                        Some(v) if !v.is_nil() => match v.as_str() {
                            Some(s) => Some(s.to_owned()),
                            None => return Err(__l.error(
                                format!("bad argument #{} ('{}': expected string)", #arg_index, #param_name)
                            )),
                        },
                        _ => None,
                    }
                }
            }
            _ => {
                // Unknown type — pass as nil → None
                quote! {
                    match __l.get_arg(#arg_index) {
                        Some(v) if !v.is_nil() => return Err(__l.error(
                            format!("bad argument #{} ('{}': unsupported type)", #arg_index, #param_name)
                        )),
                        _ => None,
                    }
                }
            }
        }
    } else {
        // Required parameter
        match type_str {
            "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
                quote! {{
                    let __v = __l.get_arg(#arg_index).unwrap_or(luars::LuaValue::nil());
                    match __v.as_integer() {
                        Some(i) => i,
                        None => match __v.as_float() {
                            Some(f) => f as i64,
                            None => return Err(__l.error(
                                format!("bad argument #{} ('{}': expected integer, got {:?})", #arg_index, #param_name, __v)
                            )),
                        }
                    }
                }}
            }
            "f32" | "f64" => {
                quote! {{
                    let __v = __l.get_arg(#arg_index).unwrap_or(luars::LuaValue::nil());
                    match __v.as_number() {
                        Some(n) => n,
                        None => return Err(__l.error(
                            format!("bad argument #{} ('{}': expected number)", #arg_index, #param_name)
                        )),
                    }
                }}
            }
            "bool" => {
                quote! {{
                    let __v = __l.get_arg(#arg_index).unwrap_or(luars::LuaValue::nil());
                    __v.as_boolean().unwrap_or(false)
                }}
            }
            "String" | "&str" => {
                quote! {{
                    let __v = __l.get_arg(#arg_index).unwrap_or(luars::LuaValue::nil());
                    match __v.as_str() {
                        Some(s) => s.to_owned(),
                        None => return Err(__l.error(
                            format!("bad argument #{} ('{}': expected string)", #arg_index, #param_name)
                        )),
                    }
                }}
            }
            _ => {
                quote! {{
                    return Err(__l.error(
                        format!("bad argument #{} ('{}': unsupported parameter type)", #arg_index, #param_name)
                    ));
                }}
            }
        }
    }
}

/// Generate code to push a Rust value onto the Lua stack and return count.
fn rust_value_push_code(
    type_str: &str,
    val_expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match type_str {
        // Unit / no return
        "()" => {
            quote! {
                let _ = #val_expr;
                Ok(0)
            }
        }
        // Integers
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
            quote! {
                __l.push_value(luars::LuaValue::integer(#val_expr as i64))?;
                Ok(1)
            }
        }
        // Floats
        "f32" | "f64" => {
            quote! {
                __l.push_value(luars::LuaValue::float(#val_expr as f64))?;
                Ok(1)
            }
        }
        // Bool
        "bool" => {
            quote! {
                __l.push_value(luars::LuaValue::boolean(#val_expr))?;
                Ok(1)
            }
        }
        // String
        "String" | "&str" => {
            quote! {
                let __s = __l.create_string(&#val_expr)?;
                __l.push_value(__s)?;
                Ok(1)
            }
        }
        // Unknown — try to convert via Into<UdValue> then push
        _ => {
            quote! {
                let __udv: luars::lua_value::userdata_trait::UdValue = #val_expr.into();
                let __lv = luars::lua_value::userdata_trait::udvalue_to_lua_value(__l, __udv)?;
                __l.push_value(__lv)?;
                Ok(1)
            }
        }
    }
}
