//! `#[derive(LuaUserData)]` — auto-generate `UserDataTrait` for Rust structs.
//!
//! Exposes public fields to Lua via `get_field` / `set_field`.
//! Methods are handled separately by `#[lua_methods]`.
//!
//! # Field attributes
//! - `#[lua(skip)]` — exclude field from Lua access
//! - `#[lua(readonly)]` — only allow get, not set
//! - `#[lua(name = "...")]` — custom Lua-visible name
//!
//! # Struct attributes
//! - `#[lua_impl(Display, PartialEq, PartialOrd)]` — map Rust traits to Lua metamethods

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Meta};

use crate::type_utils::{field_to_udvalue, udvalue_to_field};

/// Internal field metadata collected during parsing.
struct FieldInfo {
    ident: Ident,
    ty: syn::Type,
    lua_name: String,
    readonly: bool,
}

/// Entry point for `#[derive(LuaUserData)]`.
pub fn derive_lua_userdata_impl(input: DeriveInput) -> TokenStream {
    let name = &input.ident;

    // Parse #[lua_impl(...)] attribute for trait detection
    let trait_impls = parse_lua_impl_attrs(&input);

    // Handle named-field structs normally; tuple/unit structs get a minimal impl
    let fields_named = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Some(&fields.named),
            _ => None, // tuple or unit struct — no field export
        },
        Data::Enum(data) => {
            // Delegate to enum-specific codegen
            return derive_lua_enum_impl(name, data);
        }
        _ => {
            return syn::Error::new_spanned(
                &input.ident,
                "LuaUserData can only be derived for structs and C-like enums",
            )
            .to_compile_error()
            .into();
        }
    };

    // If no named fields (tuple/unit struct), generate minimal impl
    if fields_named.is_none() {
        return gen_minimal_impl(name, &trait_impls);
    }
    let fields = fields_named.unwrap();

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
    let (tostring_impl, eq_impl, ord_impl) = gen_metamethods(name, &trait_impls);

    let type_name_str = name.to_string();

    let expanded = quote! {
        impl luars::lua_value::userdata_trait::UserDataTrait for #name {
            fn type_name(&self) -> &'static str {
                #type_name_str
            }

            fn get_field(&self, key: &str) -> Option<luars::lua_value::userdata_trait::UdValue> {
                match key {
                    #(#get_field_arms)*
                    // Fall through to method lookup (inherent method from #[lua_methods]
                    // shadows the blanket LuaMethodProvider trait default)
                    _ => Self::__lua_lookup_method(key)
                        .map(luars::lua_value::userdata_trait::UdValue::Function),
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

// ==================== Metamethod generation ====================

/// Generate metamethod implementations from #[lua_impl(...)] traits.
fn gen_metamethods(
    name: &Ident,
    trait_impls: &[String],
) -> (
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
) {
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

    (tostring_impl, eq_impl, ord_impl)
}

// ==================== Minimal impl for tuple/unit structs ====================

/// Generate a minimal UserDataTrait impl for tuple or unit structs.
///
/// No field access — only type_name, method lookup, metamethods, and as_any.
fn gen_minimal_impl(name: &Ident, trait_impls: &[String]) -> TokenStream {
    let type_name_str = name.to_string();
    let (tostring_impl, eq_impl, ord_impl) = gen_metamethods(name, trait_impls);

    let expanded = quote! {
        impl luars::lua_value::userdata_trait::UserDataTrait for #name {
            fn type_name(&self) -> &'static str {
                #type_name_str
            }

            fn get_field(&self, key: &str) -> Option<luars::lua_value::userdata_trait::UdValue> {
                // No named fields — only method lookup
                Self::__lua_lookup_method(key)
                    .map(luars::lua_value::userdata_trait::UdValue::Function)
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

// ==================== Enum impl ====================

/// Generate `LuaEnum` implementation for C-like enums.
///
/// Produces a compile error if any variant has fields (data enums not supported).
/// Each variant gets its discriminant value (explicit or auto-incremented from 0).
fn derive_lua_enum_impl(name: &Ident, data: &syn::DataEnum) -> TokenStream {
    let type_name_str = name.to_string();

    // Validate: all variants must be unit (no fields)
    for variant in &data.variants {
        if !variant.fields.is_empty() {
            return syn::Error::new_spanned(
                &variant.ident,
                "LuaUserData enum export only supports C-like enums (no fields on variants)",
            )
            .to_compile_error()
            .into();
        }
    }

    // Collect variant names and discriminant values
    let mut entries = Vec::new();
    let mut next_discriminant: i64 = 0;

    for variant in &data.variants {
        let variant_name = variant.ident.to_string();
        let disc_value = if let Some((_, expr)) = &variant.discriminant {
            match parse_discriminant_expr(expr) {
                Ok(v) => {
                    next_discriminant = v + 1;
                    v
                }
                Err(e) => return e.to_compile_error().into(),
            }
        } else {
            let v = next_discriminant;
            next_discriminant += 1;
            v
        };

        entries.push((variant_name, disc_value));
    }

    let variant_names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    let variant_values: Vec<i64> = entries.iter().map(|(_, v)| *v).collect();

    let expanded = quote! {
        impl luars::lua_value::userdata_trait::LuaEnum for #name {
            fn variants() -> &'static [(&'static str, i64)] {
                &[#( (#variant_names, #variant_values) ),*]
            }

            fn enum_name() -> &'static str {
                #type_name_str
            }
        }
    };

    expanded.into()
}

/// Parse an enum discriminant expression to i64.
fn parse_discriminant_expr(expr: &syn::Expr) -> Result<i64, syn::Error> {
    match expr {
        syn::Expr::Lit(lit) => {
            if let syn::Lit::Int(int_lit) = &lit.lit {
                int_lit.base10_parse::<i64>().map_err(|_| {
                    syn::Error::new_spanned(int_lit, "enum discriminant must be a valid i64")
                })
            } else {
                Err(syn::Error::new_spanned(
                    expr,
                    "enum discriminant must be an integer literal",
                ))
            }
        }
        syn::Expr::Unary(unary) => {
            if let syn::UnOp::Neg(_) = unary.op {
                let val = parse_discriminant_expr(&unary.expr)?;
                Ok(-val)
            } else {
                Err(syn::Error::new_spanned(
                    expr,
                    "enum discriminant must be an integer literal",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "enum discriminant must be an integer literal",
        )),
    }
}
