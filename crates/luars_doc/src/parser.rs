use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use syn::{
    Attribute, FnArg, ImplItem, Item, ItemEnum, ItemImpl, ItemStruct, Meta, Pat, ReturnType, Type,
    Visibility,
};

use crate::emit::{DocBundle, FieldInfo, MethodInfo, ParamInfo, UserdataInfo, rust_type_to_lua};

/// Parse one or more `.rs` files and collect userdata doc information.
pub(crate) fn parse_files(paths: &[impl AsRef<Path>]) -> Result<DocBundle> {
    let mut bundle = DocBundle::default();

    for path in paths {
        let path = path.as_ref();
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let file = syn::parse_file(&source)
            .with_context(|| format!("failed to parse {}", path.display()))?;

        let file_docs = extract_file_userdata_types(&file.items);
        bundle.types.extend(file_docs);
    }

    Ok(bundle)
}

/// Recursively walk a directory for `.rs` files and parse them.
pub(crate) fn parse_dir(dir: &Path) -> Result<DocBundle> {
    if !dir.is_dir() {
        bail!("directory not found: {}", dir.display());
    }

    let mut paths: Vec<std::path::PathBuf> = Vec::new();

    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            paths.push(path.to_path_buf());
        }
    }

    if paths.is_empty() {
        bail!("no .rs files found in {}", dir.display());
    }

    let refs: Vec<&std::path::PathBuf> = paths.iter().collect();
    let bundle = parse_files(&refs)?;

    if bundle.types.is_empty() {
        eprintln!(
            "warning: no #[derive(LuaUserData)] types found in {}",
            dir.display()
        );
    }

    Ok(bundle)
}

// ── file-level extraction ────────────────────────────────────────────────

fn extract_file_userdata_types(items: &[Item]) -> Vec<UserdataInfo> {
    let mut userdata_names: HashMap<String, UserdataInfo> = HashMap::new();
    collect_userdata_types(items, &mut userdata_names);
    userdata_names.into_values().collect()
}

fn collect_userdata_types(items: &[Item], userdata_names: &mut HashMap<String, UserdataInfo>) {
    for item in items {
        match item {
            // Structs
            Item::Struct(s) if has_lua_userdata_derive(&s.attrs) => {
                let info = parse_struct_userdata(s);
                userdata_names.insert(info.name.clone(), info);
            }
            // Enums
            Item::Enum(e) if has_lua_userdata_derive(&e.attrs) => {
                let info = parse_enum_userdata(e);
                userdata_names.insert(info.name.clone(), info);
            }
            // #[lua_methods] impl blocks
            Item::Impl(imp) if has_lua_methods_attr(&imp.attrs) => {
                let self_ty_name = type_to_name(&imp.self_ty);
                if let Some(info) = userdata_names.get_mut(&self_ty_name) {
                    let methods = parse_lua_methods_impl(imp, &info.name);
                    info.methods.extend(methods);
                }
            }
            // Recurse into inline modules (mod tests { ... })
            Item::Mod(m) if m.content.is_some() => {
                let (_, items) = m.content.as_ref().unwrap();
                collect_userdata_types(items, userdata_names);
            }
            _ => {}
        }
    }
}

// ── attribute helpers ────────────────────────────────────────────────────

fn has_lua_userdata_derive(attrs: &[Attribute]) -> bool {
    for attr in attrs {
        if attr.path().is_ident("derive") {
            let s = quote::quote!(#attr).to_string();
            if s.contains("LuaUserData") {
                return true;
            }
        }
    }
    false
}

fn has_lua_methods_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("lua_methods"))
}

fn type_to_name(ty: &Type) -> String {
    if let Type::Path(tp) = ty {
        tp.path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default()
    } else {
        quote::quote!(#ty).to_string()
    }
}

// ── struct / enum parsing ────────────────────────────────────────────────

fn parse_struct_userdata(s: &ItemStruct) -> UserdataInfo {
    let name = s.ident.to_string();
    let doc = extract_doc(&s.attrs);

    let (trait_impls, delegates) = parse_lua_impl_and_delegate_attrs(&s.attrs);

    let fields = match &s.fields {
        syn::Fields::Named(fields) => fields.named.iter().filter_map(parse_field_info).collect(),
        _ => Vec::new(),
    };

    UserdataInfo {
        name,
        doc,
        fields,
        methods: Vec::new(),
        metamethods: trait_impls,
        delegates,
        super_class: None,
    }
}

fn parse_enum_userdata(e: &ItemEnum) -> UserdataInfo {
    let name = e.ident.to_string();
    let doc = extract_doc(&e.attrs);
    let (trait_impls, delegates) = parse_lua_impl_and_delegate_attrs(&e.attrs);

    UserdataInfo {
        name,
        doc,
        fields: Vec::new(),
        methods: Vec::new(),
        metamethods: trait_impls,
        delegates,
        super_class: None,
    }
}

fn type_to_string(ty: &Type) -> String {
    quote::quote!(#ty).to_string().replace(' ', "")
}

fn parse_field_info(field: &syn::Field) -> Option<FieldInfo> {
    let ident = field.ident.as_ref()?;

    // Only expose public fields
    if !matches!(field.vis, Visibility::Public(_)) {
        return None;
    }

    let raw_ty = type_to_string(&field.ty);
    let doc = extract_doc(&field.attrs);

    let mut skip = false;
    let mut readonly = false;
    let mut lua_name = ident.to_string();

    for attr in &field.attrs {
        if attr.path().is_ident("lua")
            && let Ok(list) = attr.meta.require_list()
        {
            let _ = list.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    skip = true;
                } else if meta.path.is_ident("readonly") {
                    readonly = true;
                } else if meta.path.is_ident("name")
                    && let Ok(value) = meta.value()
                    && let Ok(lit) = value.parse::<syn::LitStr>()
                {
                    lua_name = lit.value();
                }
                Ok(())
            });
        }
    }

    if skip {
        return None;
    }

    Some(FieldInfo {
        lua_name,
        lua_ty: rust_type_to_lua(&raw_ty, ""),
        rust_ty: raw_ty,
        readonly,
        doc,
    })
}

fn parse_lua_impl_and_delegate_attrs(attrs: &[Attribute]) -> (Vec<String>, Vec<(String, String)>) {
    let mut trait_impls = Vec::new();
    let mut delegates = Vec::new();

    for attr in attrs {
        // #[lua_impl(Display, PartialEq, ...)]
        if attr.path().is_ident("lua_impl")
            && let Meta::List(list) = &attr.meta
        {
            let _ = list.parse_nested_meta(|meta| {
                if let Some(ident) = meta.path.get_ident() {
                    trait_impls.push(ident.to_string());
                }
                Ok(())
            });
        }

        // #[lua(close = "...", pow = "...")]
        if attr.path().is_ident("lua")
            && let Ok(list) = attr.meta.require_list()
        {
            let _ = list.parse_nested_meta(|meta| {
                let key = meta
                    .path
                    .get_ident()
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                if let Ok(value) = meta.value()
                    && let Ok(lit) = value.parse::<syn::LitStr>()
                {
                    delegates.push((key, lit.value()));
                }
                Ok(())
            });
        }
    }

    (trait_impls, delegates)
}

// ── method parsing ───────────────────────────────────────────────────────

fn parse_lua_methods_impl(imp: &ItemImpl, self_name: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();

    for item in &imp.items {
        if let ImplItem::Fn(method) = item {
            if !matches!(method.vis, Visibility::Public(_)) {
                continue;
            }
            if method.sig.asyncness.is_some() {
                continue;
            }

            let mut skip = false;
            let mut lua_name_override: Option<String> = None;
            for attr in &method.attrs {
                if attr.path().is_ident("lua")
                    && let Ok(list) = attr.meta.require_list()
                {
                    let _ = list.parse_nested_meta(|meta| {
                        if meta.path.is_ident("skip") {
                            skip = true;
                        } else if meta.path.is_ident("name")
                            && let Ok(value) = meta.value()
                            && let Ok(lit) = value.parse::<syn::LitStr>()
                        {
                            lua_name_override = Some(lit.value());
                        }
                        Ok(())
                    });
                }
            }
            if skip {
                continue;
            }

            let sig = &method.sig;
            let rust_name = sig.ident.to_string();
            let lua_name = lua_name_override.unwrap_or_else(|| rust_name.clone());
            let doc = extract_doc(&method.attrs);

            // Determine kind
            let (is_static, is_mut) = match sig.inputs.first() {
                Some(FnArg::Receiver(r)) => (false, r.mutability.is_some()),
                _ => (true, false),
            };

            // Parse params (skip self)
            let skip_count: usize = if is_static { 0 } else { 1 };
            let mut params = Vec::new();
            for arg in sig.inputs.iter().skip(skip_count) {
                if let FnArg::Typed(pat_type) = arg {
                    let param_name = if let Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                        pat_ident.ident.to_string()
                    } else {
                        "__arg".to_string()
                    };
                    let rust_ty = type_to_string(&pat_type.ty);
                    let lua_ty = rust_type_to_lua(&rust_ty, self_name);
                    params.push(ParamInfo {
                        name: param_name,
                        lua_ty,
                    });
                }
            }

            // Return type
            let return_type = match &sig.output {
                ReturnType::Default => None,
                ReturnType::Type(_, ret_ty) => {
                    let rust_ty = type_to_string(ret_ty);
                    if rust_ty == "()" {
                        None
                    } else {
                        Some(rust_type_to_lua(&rust_ty, self_name))
                    }
                }
            };

            methods.push(MethodInfo {
                lua_name,
                is_static,
                is_mut,
                params,
                return_type,
                doc,
            });
        }
    }

    methods
}

// ── doc comment extraction ───────────────────────────────────────────────

fn extract_doc(attrs: &[Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter_map(|a| {
            if a.path().is_ident("doc")
                && let Meta::NameValue(nv) = &a.meta
                && let syn::Expr::Lit(lit) = &nv.value
                && let syn::Lit::Str(s) = &lit.lit
            {
                return Some(s.value().trim().to_string());
            }
            None
        })
        .collect();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}
