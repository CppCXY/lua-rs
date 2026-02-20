//! `#[lua_methods]` — attribute macro for impl blocks.
//!
//! Generates static C wrapper functions for each `pub fn` in the impl block:
//!
//! - **Instance methods** (`&self` / `&mut self`) → accessible via `obj:method(args)`
//! - **Associated functions** (no `self`, e.g. `new`) → accessible via `Type.func(args)`
//!
//! # Instance methods
//!
//! For each `pub fn method(&self, ...)` or `pub fn method(&mut self, ...)`:
//! 1. A static wrapper `fn(l: &mut LuaState) -> LuaResult<usize>` is generated
//! 2. The wrapper extracts `self` from arg 1 (userdata), converts remaining args
//! 3. The return value is converted back to Lua
//!
//! # Associated functions (constructors / static methods)
//!
//! For each `pub fn name(args...) -> ...` without `self`:
//! 1. A static wrapper is generated
//! 2. If the return type is `Self`, the result is wrapped in `LuaUserdata::new()`
//!    and pushed as userdata
//! 3. Registered via `__lua_static_methods()` for use with `register_type`
//!
//! # Supported parameter types
//! - `i8..i64`, `u8..u64`, `isize`, `usize` — extracted via `as_integer()`
//! - `f32`, `f64` — extracted via `as_number()`
//! - `bool` — extracted via `as_boolean()`
//! - `String`, `&str` — extracted via `as_str()`
//! - `Option<T>` — nil/missing → `None`
//!
//! # Supported return types
//! - `()` — returns 0 values
//! - Numeric, bool, String — pushed as single value
//! - `Option<T>` — `None` → nil
//! - `Result<T, E>` — `Err` → Lua error
//! - `Self` — wrapped in `LuaUserdata::new()`, pushed as userdata
//!
//! # Example
//! ```ignore
//! #[lua_methods]
//! impl Point {
//!     pub fn new(x: f64, y: f64) -> Self {
//!         Point { x, y }
//!     }
//!     pub fn distance(&self) -> f64 {
//!         (self.x * self.x + self.y * self.y).sqrt()
//!     }
//!     pub fn translate(&mut self, dx: f64, dy: f64) {
//!         self.x += dx;
//!         self.y += dy;
//!     }
//! }
//! ```

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemImpl, Pat, ReturnType, parse_macro_input};

use crate::type_utils::normalize_type;

/// Kind of method discovered in the impl block.
enum MethodKind {
    /// `&self` method
    Ref,
    /// `&mut self` method
    RefMut,
    /// Associated function (no self)
    Static,
}

/// Information about a single method to be wrapped.
struct MethodInfo {
    /// Rust method name
    rust_name: syn::Ident,
    /// Lua-visible name (same as rust_name unless overridden)
    lua_name: String,
    /// Kind of method
    kind: MethodKind,
    /// Parameter names and types (excluding self)
    params: Vec<(syn::Ident, syn::Type)>,
    /// Return type (None = unit)
    return_type: Option<syn::Type>,
}

/// Entry point for `#[lua_methods]`.
pub fn lua_methods_impl(input: TokenStream) -> TokenStream {
    let item_impl = parse_macro_input!(input as ItemImpl);

    // Extract the struct name from the impl block
    let self_ty = &item_impl.self_ty;

    // Collect method info from all pub methods
    let mut methods: Vec<MethodInfo> = Vec::new();

    for item in &item_impl.items {
        if let syn::ImplItem::Fn(method) = item {
            // Only process public methods
            if !matches!(method.vis, syn::Visibility::Public(_)) {
                continue;
            }

            let sig = &method.sig;

            // Skip async methods
            if sig.asyncness.is_some() {
                continue;
            }

            // Parse #[lua(...)] attributes on this method
            let mut skip = false;
            let mut lua_name_override: Option<String> = None;
            for attr in &method.attrs {
                if attr.path().is_ident("lua") {
                    if let Ok(list) = attr.meta.require_list() {
                        let _ = list.parse_nested_meta(|meta| {
                            if meta.path.is_ident("skip") {
                                skip = true;
                            } else if meta.path.is_ident("name") {
                                if let Ok(value) = meta.value() {
                                    if let Ok(lit) = value.parse::<syn::LitStr>() {
                                        lua_name_override = Some(lit.value());
                                    }
                                }
                            }
                            Ok(())
                        });
                    }
                }
            }

            if skip {
                continue;
            }

            // Determine method kind
            let kind = match sig.inputs.first() {
                Some(FnArg::Receiver(r)) => {
                    if r.mutability.is_some() {
                        MethodKind::RefMut
                    } else {
                        MethodKind::Ref
                    }
                }
                _ => MethodKind::Static, // no self → associated function
            };

            // Collect non-self parameters
            let skip_count = match &kind {
                MethodKind::Static => 0,
                _ => 1,
            };
            let mut params = Vec::new();
            for arg in sig.inputs.iter().skip(skip_count) {
                if let FnArg::Typed(pat_type) = arg {
                    let param_name = if let Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                        pat_ident.ident.clone()
                    } else {
                        format_ident!("__arg")
                    };
                    params.push((param_name, (*pat_type.ty).clone()));
                }
            }

            // Extract return type
            let return_type = match &sig.output {
                ReturnType::Default => None,
                ReturnType::Type(_, ty) => Some((**ty).clone()),
            };

            let rust_name = sig.ident.clone();
            let lua_name = lua_name_override.unwrap_or_else(|| rust_name.to_string());

            methods.push(MethodInfo {
                rust_name,
                lua_name,
                kind,
                params,
                return_type,
            });
        }
    }

    // Split into instance methods and static methods
    let instance_methods: Vec<&MethodInfo> = methods
        .iter()
        .filter(|m| !matches!(m.kind, MethodKind::Static))
        .collect();
    let static_methods: Vec<&MethodInfo> = methods
        .iter()
        .filter(|m| matches!(m.kind, MethodKind::Static))
        .collect();

    // Create a cleaned copy of the impl block with #[lua(...)] attributes stripped
    let mut cleaned_impl = item_impl.clone();
    for item in &mut cleaned_impl.items {
        if let syn::ImplItem::Fn(method) = item {
            method.attrs.retain(|attr| !attr.path().is_ident("lua"));
        }
    }

    // Generate wrapper functions for instance methods
    let instance_wrapper_fns: Vec<proc_macro2::TokenStream> = instance_methods
        .iter()
        .map(|m| gen_instance_wrapper_fn(self_ty, m))
        .collect();

    // Generate __lua_lookup_method match arms
    let lookup_arms: Vec<proc_macro2::TokenStream> = instance_methods
        .iter()
        .map(|m| {
            let lua_name = &m.lua_name;
            let wrapper_name = format_ident!("__lua_method_{}", m.rust_name);
            quote! { #lua_name => Some(#wrapper_name), }
        })
        .collect();

    // Generate wrapper functions for static methods
    let static_wrapper_fns: Vec<proc_macro2::TokenStream> = static_methods
        .iter()
        .map(|m| gen_static_wrapper_fn(self_ty, m))
        .collect();

    // Generate static methods table entries
    let static_entries: Vec<proc_macro2::TokenStream> = static_methods
        .iter()
        .map(|m| {
            let lua_name = &m.lua_name;
            let wrapper_name = format_ident!("__lua_static_{}", m.rust_name);
            quote! { (#lua_name, #wrapper_name as luars::lua_vm::CFunction), }
        })
        .collect();

    let expanded = quote! {
        // Re-emit the original impl block with #[lua(...)] attributes stripped
        #cleaned_impl

        // Additional impl block with lookup + static method registration
        impl #self_ty {
            /// Look up a Lua-callable wrapper function by instance method name.
            ///
            /// This inherent method shadows the blanket `LuaMethodProvider` trait default,
            /// so `#[derive(LuaUserData)]`'s `get_field` will find methods here.
            #[allow(unused)]
            pub fn __lua_lookup_method(key: &str) -> Option<luars::lua_vm::CFunction> {
                #(#instance_wrapper_fns)*

                match key {
                    #(#lookup_arms)*
                    _ => None,
                }
            }

            /// Return all static (associated) methods for type registration.
            ///
            /// This inherent method shadows the blanket `LuaStaticMethodProvider`
            /// trait default, providing entries for `register_type` to populate
            /// the class table (e.g. `Point.new`).
            #[allow(unused)]
            pub fn __lua_static_methods() -> &'static [(&'static str, luars::lua_vm::CFunction)] {
                #(#static_wrapper_fns)*

                &[#(#static_entries)*]
            }
        }

        // Explicit trait impl for LuaRegistrable — enables register_type_of::<T>.
        // Unlike the blanket LuaStaticMethodProvider, this dispatches to the
        // actual generated methods rather than returning &[].
        impl luars::LuaRegistrable for #self_ty {
            fn lua_static_methods() -> &'static [(&'static str, luars::lua_vm::CFunction)] {
                #self_ty::__lua_static_methods()
            }
        }
    };

    expanded.into()
}

/// Generate a static wrapper function for an instance method (`&self` / `&mut self`).
fn gen_instance_wrapper_fn(self_ty: &syn::Type, method: &MethodInfo) -> proc_macro2::TokenStream {
    let wrapper_name = format_ident!("__lua_method_{}", method.rust_name);
    let rust_name = &method.rust_name;

    // Arg 1 = self (userdata), user params start at arg 2
    let param_extractions: Vec<proc_macro2::TokenStream> = method
        .params
        .iter()
        .enumerate()
        .map(|(i, (name, ty))| {
            let arg_index = i + 2;
            let param_name_str = name.to_string();
            gen_from_lua_extraction(name, ty, arg_index, &param_name_str)
        })
        .collect();

    let param_names: Vec<&syn::Ident> = method.params.iter().map(|(name, _)| name).collect();

    let call_and_return = match &method.kind {
        MethodKind::RefMut => gen_mut_call(self_ty, rust_name, &param_names, &method.return_type),
        _ => gen_ref_call(self_ty, rust_name, &param_names, &method.return_type),
    };

    quote! {
        fn #wrapper_name(__l: &mut luars::lua_vm::LuaState) -> luars::lua_vm::LuaResult<usize> {
            #(#param_extractions)*
            #call_and_return
        }
    }
}

/// Generate a static wrapper function for an associated function (no `self`).
fn gen_static_wrapper_fn(self_ty: &syn::Type, method: &MethodInfo) -> proc_macro2::TokenStream {
    let wrapper_name = format_ident!("__lua_static_{}", method.rust_name);
    let rust_name = &method.rust_name;

    // Static functions: params start at arg 1 (no self)
    let param_extractions: Vec<proc_macro2::TokenStream> = method
        .params
        .iter()
        .enumerate()
        .map(|(i, (name, ty))| {
            let arg_index = i + 1; // 1-based, no self to skip
            let param_name_str = name.to_string();
            gen_from_lua_extraction(name, ty, arg_index, &param_name_str)
        })
        .collect();

    let param_names: Vec<&syn::Ident> = method.params.iter().map(|(name, _)| name).collect();

    let call_and_push = gen_static_call(self_ty, rust_name, &param_names, &method.return_type);

    quote! {
        fn #wrapper_name(__l: &mut luars::lua_vm::LuaState) -> luars::lua_vm::LuaResult<usize> {
            #(#param_extractions)*
            #call_and_push
        }
    }
}

// ==================== Trait-based codegen helpers ====================

/// Generate code to extract a parameter from a Lua argument using `FromLua`.
///
/// For `&str` parameters, generates `String` extraction then borrows.
/// For all other types, generates a direct `FromLua::from_lua()` call.
fn gen_from_lua_extraction(
    name: &syn::Ident,
    ty: &syn::Type,
    arg_index: usize,
    param_name: &str,
) -> proc_macro2::TokenStream {
    let type_str = normalize_type(ty);

    if type_str == "&str" {
        // &str cannot implement FromLua (not Sized + lifetime).
        // Extract as String, then the call site uses &name.
        let storage_name = format_ident!("__{}_storage", name);
        quote! {
            let #storage_name: String = {
                let __v = __l.get_arg(#arg_index).unwrap_or(luars::LuaValue::nil());
                <String as luars::FromLua>::from_lua(__v, __l)
                    .map_err(|e| __l.error(format!("bad argument #{} '{}': {}", #arg_index, #param_name, e)))?
            };
            let #name: &str = &#storage_name;
        }
    } else {
        quote! {
            let #name: #ty = {
                let __v = __l.get_arg(#arg_index).unwrap_or(luars::LuaValue::nil());
                <#ty as luars::FromLua>::from_lua(__v, __l)
                    .map_err(|e| __l.error(format!("bad argument #{} '{}': {}", #arg_index, #param_name, e)))?
            };
        }
    }
}

/// Generate code to push a return value onto the Lua stack using `IntoLua`.
///
/// Special cases:
/// - `Self` → wrap in `LuaUserdata::new()` and push as userdata
/// - All other types → `IntoLua::into_lua(result, state)`
fn gen_return_push(
    _self_ty: &syn::Type,
    return_type: &Option<syn::Type>,
) -> proc_macro2::TokenStream {
    match return_type {
        None => {
            // Unit return → 0 values
            quote! { Ok(0) }
        }
        Some(ret_ty) => {
            let type_str = normalize_type(ret_ty);
            if type_str == "Self" {
                // Self → wrap in LuaUserdata and push
                quote! {
                    let __ud = luars::LuaUserdata::new(__result);
                    let __ud_val = __l.create_userdata(__ud)?;
                    __l.push_value(__ud_val)?;
                    Ok(1)
                }
            } else {
                // Use IntoLua trait — handles Result<T,E>, Option<T>, primitives, String, etc.
                quote! {
                    luars::IntoLua::into_lua(__result, __l)
                        .map_err(|e| __l.error(e))
                }
            }
        }
    }
}

/// Generate code for calling a static/associated function.
fn gen_static_call(
    self_ty: &syn::Type,
    method_name: &syn::Ident,
    param_names: &[&syn::Ident],
    return_type: &Option<syn::Type>,
) -> proc_macro2::TokenStream {
    let push = gen_return_push(self_ty, return_type);
    match return_type {
        None => {
            quote! {
                #self_ty::#method_name(#(#param_names),*);
                #push
            }
        }
        Some(_) => {
            quote! {
                let __result = #self_ty::#method_name(#(#param_names),*);
                #push
            }
        }
    }
}

/// Generate code for calling a `&self` method.
fn gen_ref_call(
    self_ty: &syn::Type,
    method_name: &syn::Ident,
    param_names: &[&syn::Ident],
    return_type: &Option<syn::Type>,
) -> proc_macro2::TokenStream {
    let type_name = quote!(#self_ty).to_string();
    let method_name_str = method_name.to_string();
    let push = gen_return_push(self_ty, return_type);

    match return_type {
        None => {
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_ref::<#self_ty>() {
                        __this.#method_name(#(#param_names),*);
                        return #push;
                    }
                }
                Err(__l.error(format!("{}:{} — invalid self", #type_name, #method_name_str)))
            }
        }
        Some(_) => {
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_ref::<#self_ty>() {
                        let __result = __this.#method_name(#(#param_names),*);
                        return { #push };
                    }
                }
                Err(__l.error(format!("{}:{} — invalid self", #type_name, #method_name_str)))
            }
        }
    }
}

/// Generate code for calling a `&mut self` method.
fn gen_mut_call(
    self_ty: &syn::Type,
    method_name: &syn::Ident,
    param_names: &[&syn::Ident],
    return_type: &Option<syn::Type>,
) -> proc_macro2::TokenStream {
    let type_name = quote!(#self_ty).to_string();
    let method_name_str = method_name.to_string();
    let push = gen_return_push(self_ty, return_type);

    match return_type {
        None => {
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_mut::<#self_ty>() {
                        __this.#method_name(#(#param_names),*);
                        return #push;
                    }
                }
                Err(__l.error(format!("{}:{} — invalid self", #type_name, #method_name_str)))
            }
        }
        Some(_) => {
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_mut::<#self_ty>() {
                        let __result = __this.#method_name(#(#param_names),*);
                        return { #push };
                    }
                }
                Err(__l.error(format!("{}:{} — invalid self", #type_name, #method_name_str)))
            }
        }
    }
}
