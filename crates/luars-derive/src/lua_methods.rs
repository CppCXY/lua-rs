//! `#[lua_methods]` — attribute macro for impl blocks.
//!
//! Generates static C wrapper functions for each `pub fn` in the impl block,
//! and a `__lua_lookup_method(key) -> Option<CFunction>` inherent method
//! that maps Lua method names to those wrappers.
//!
//! # How it works
//!
//! For each `pub fn method_name(&self, ...)` or `pub fn method_name(&mut self, ...)`:
//! 1. A static `fn __lua_method_<name>(l: &mut LuaState) -> LuaResult<usize>` is generated
//! 2. The wrapper extracts `self` from arg 1 (userdata), converts remaining args, calls the method
//! 3. The return value is converted back to Lua values
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
//!
//! # Example
//! ```ignore
//! #[lua_methods]
//! impl Point {
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

use crate::type_utils::{lua_arg_to_rust, rust_return_to_lua};

/// Information about a single method to be wrapped.
struct MethodInfo {
    /// Rust method name
    rust_name: syn::Ident,
    /// Lua-visible name (same as rust_name unless overridden)
    lua_name: String,
    /// Whether the method takes `&mut self` (vs `&self`)
    is_mut: bool,
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

            // Must have a self parameter
            let first_arg = match sig.inputs.first() {
                Some(FnArg::Receiver(r)) => r,
                _ => continue, // skip associated functions (no self)
            };

            let is_mut = first_arg.mutability.is_some();

            // Collect non-self parameters
            let mut params = Vec::new();
            for arg in sig.inputs.iter().skip(1) {
                if let FnArg::Typed(pat_type) = arg {
                    let param_name = if let Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                        pat_ident.ident.clone()
                    } else {
                        // Fallback: use a generated name
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
            let lua_name = rust_name.to_string();

            methods.push(MethodInfo {
                rust_name,
                lua_name,
                is_mut,
                params,
                return_type,
            });
        }
    }

    // Generate static wrapper functions
    let wrapper_fns: Vec<proc_macro2::TokenStream> = methods.iter().map(|m| {
        gen_wrapper_fn(self_ty, m)
    }).collect();

    // Generate __lua_lookup_method match arms
    let lookup_arms: Vec<proc_macro2::TokenStream> = methods.iter().map(|m| {
        let lua_name = &m.lua_name;
        let wrapper_name = format_ident!("__lua_method_{}", m.rust_name);
        quote! { #lua_name => Some(#wrapper_name), }
    }).collect();

    let expanded = quote! {
        // Re-emit the original impl block unchanged
        #item_impl

        // Additional impl block with lookup method + wrapper functions
        impl #self_ty {
            /// Look up a Lua-callable wrapper function by method name.
            ///
            /// This inherent method shadows the blanket `LuaMethodProvider` trait default,
            /// so `#[derive(LuaUserData)]`'s `get_field` will find methods here.
            #[allow(unused)]
            pub fn __lua_lookup_method(key: &str) -> Option<luars::lua_vm::CFunction> {
                // Static wrapper functions (defined below)
                #(#wrapper_fns)*

                match key {
                    #(#lookup_arms)*
                    _ => None,
                }
            }
        }
    };

    expanded.into()
}

/// Generate a static wrapper function for a single method.
fn gen_wrapper_fn(
    self_ty: &syn::Type,
    method: &MethodInfo,
) -> proc_macro2::TokenStream {
    let wrapper_name = format_ident!("__lua_method_{}", method.rust_name);
    let rust_name = &method.rust_name;

    // Generate arg extraction code for each parameter
    // Arg 1 = self (userdata), user params start at arg 2
    let param_extractions: Vec<proc_macro2::TokenStream> = method.params.iter().enumerate().map(|(i, (name, ty))| {
        let arg_index = i + 2; // 1-based, skip self
        let param_name_str = name.to_string();
        let extract = lua_arg_to_rust(ty, arg_index, &param_name_str);
        quote! { let #name = #extract; }
    }).collect();

    // Generate the method call
    let param_names: Vec<&syn::Ident> = method.params.iter().map(|(name, _)| name).collect();

    // Generate the method call with proper self borrowing
    let call_and_return = if method.is_mut {
        gen_mut_call(self_ty, rust_name, &param_names, &method.return_type)
    } else {
        gen_ref_call(self_ty, rust_name, &param_names, &method.return_type)
    };

    quote! {
        fn #wrapper_name(__l: &mut luars::lua_vm::LuaState) -> luars::lua_vm::LuaResult<usize> {
            #(#param_extractions)*
            #call_and_return
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

    match return_type {
        None => {
            // No return value
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_ref::<#self_ty>() {
                        __this.#method_name(#(#param_names),*);
                        return Ok(0);
                    }
                }
                Err(__l.error(format!("{}:{} — invalid self", #type_name, #method_name_str)))
            }
        }
        Some(ret_ty) => {
            let push_result = rust_return_to_lua(ret_ty);
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_ref::<#self_ty>() {
                        let __result = __this.#method_name(#(#param_names),*);
                        return { #push_result };
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

    match return_type {
        None => {
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_mut::<#self_ty>() {
                        __this.#method_name(#(#param_names),*);
                        return Ok(0);
                    }
                }
                Err(__l.error(format!("{}:{} — invalid self", #type_name, #method_name_str)))
            }
        }
        Some(ret_ty) => {
            let push_result = rust_return_to_lua(ret_ty);
            quote! {
                let __self_val = __l.get_arg(1)
                    .ok_or_else(|| __l.error(format!("{}:{} — missing self argument", #type_name, #method_name_str)))?;
                if let Some(__ud) = __self_val.as_userdata_mut() {
                    if let Some(__this) = __ud.downcast_mut::<#self_ty>() {
                        let __result = __this.#method_name(#(#param_names),*);
                        return { #push_result };
                    }
                }
                Err(__l.error(format!("{}:{} — invalid self", #type_name, #method_name_str)))
            }
        }
    }
}
