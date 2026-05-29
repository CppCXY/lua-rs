use std::fmt::Write;

/// Collected userdata API information parsed from Rust source files.
#[derive(Debug, Default)]
pub(crate) struct DocBundle {
    pub types: Vec<UserdataInfo>,
}

#[derive(Debug, Clone)]
pub(crate) struct UserdataInfo {
    pub name: String,
    pub doc: Option<String>,
    pub fields: Vec<FieldInfo>,
    pub methods: Vec<MethodInfo>,
    /// Raw trait names from `#[lua_impl(Display, PartialEq, ...)]`.
    pub metamethods: Vec<String>,
    pub delegates: Vec<(String, String)>,
    pub super_class: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct FieldInfo {
    pub lua_name: String,
    pub rust_ty: String,
    pub lua_ty: String,
    pub readonly: bool,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct MethodInfo {
    pub lua_name: String,
    pub is_static: bool,
    pub is_mut: bool,
    pub params: Vec<ParamInfo>,
    pub return_type: Option<String>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParamInfo {
    pub name: String,
    pub lua_ty: String,
}

// ── Rust → Lua type mapping ─────────────────────────────────────────────

pub(crate) fn rust_type_to_lua(ty: &str, self_name: &str) -> String {
    let ty = ty.trim();
    match ty {
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
            "integer".into()
        }
        "f32" | "f64" => "number".into(),
        "bool" => "boolean".into(),
        "String" | "&str" | "&'static str" => "string".into(),
        "Self" => self_name.into(),
        _ if ty.starts_with("Result<") => {
            let params = extract_two_params(ty, "Result");
            rust_type_to_lua(&params.0, self_name)
        }
        _ if ty.starts_with("Vec<") => {
            let inner = extract_generic_param(ty, "Vec");
            format!("{}[]", rust_type_to_lua(&inner, self_name))
        }
        _ if ty.starts_with("Option<") => {
            let inner = extract_generic_param(ty, "Option");
            format!("{}|nil", rust_type_to_lua(&inner, self_name))
        }
        _ if ty.starts_with("HashMap<") => {
            let params = extract_two_params(ty, "HashMap");
            format!(
                "table<{}, {}>",
                rust_type_to_lua(&params.0, self_name),
                rust_type_to_lua(&params.1, self_name),
            )
        }
        _ => ty.to_string(),
    }
}

fn extract_generic_param(ty: &str, container: &str) -> String {
    let start = container.len() + 1;
    let end = ty.trim_end_matches('>').len();
    ty[start..end].trim().to_string()
}

fn extract_two_params(ty: &str, container: &str) -> (String, String) {
    let inner = &ty[container.len() + 1..ty.len() - 1];
    let mut depth = 0;
    let mut split = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                split = i;
                break;
            }
            _ => {}
        }
    }
    let a = inner[..split].trim().to_string();
    let b = inner[split + 1..].trim().to_string();
    (a, b)
}

// ── EmmyLua operator mapping ────────────────────────────────────────────

/// Map a `#[lua_impl(Trait)]` name to an `@operator` annotation.
/// Returns `None` for traits that EmmyLua does not support (e.g. bitwise).
fn trait_to_operator(trait_name: &str, class_name: &str) -> Option<String> {
    match trait_name {
        "Add" => Some(format!("---@operator add({class_name}): {class_name}")),
        "Sub" => Some(format!("---@operator sub({class_name}): {class_name}")),
        "Mul" => Some(format!("---@operator mul({class_name}): {class_name}")),
        "Div" => Some(format!("---@operator div({class_name}): {class_name}")),
        "Rem" => Some(format!("---@operator mod({class_name}): {class_name}")),
        "Neg" => Some(format!("---@operator unm: {class_name}")),
        "Pow" => Some(format!("---@operator pow({class_name}): {class_name}")),
        "PartialEq" => Some(format!("---@operator eq({class_name}): boolean")),
        "PartialOrd" => {
            // lt and le are separate; handled specially in emit
            None
        }
        // Not EmmyLua operators — skip
        "Not" | "BitAnd" | "BitOr" | "BitXor" | "Shl" | "Shr" => None,
        // Display → __tostring, handled as a field
        _ => None,
    }
}

// ── EmmyLua emitter ─────────────────────────────────────────────────────

pub(crate) fn emit_lua(bundle: &DocBundle) -> String {
    let mut out = String::from("---@meta\n\n");

    for ud in &bundle.types {
        emit_userdata(&mut out, ud);
        out.push('\n');
    }

    out
}

fn emit_userdata(out: &mut String, ud: &UserdataInfo) {
    // class doc comment
    if let Some(ref doc) = ud.doc {
        for line in doc.lines() {
            writeln!(out, "--- {}", line).unwrap();
        }
    }

    // ---@class
    match &ud.super_class {
        Some(parent) => writeln!(out, "---@class {} : {}", ud.name, parent).unwrap(),
        None => writeln!(out, "---@class {}", ud.name).unwrap(),
    }

    // fields
    for field in &ud.fields {
        let ro = if field.readonly { " (readonly)" } else { "" };
        match &field.doc {
            Some(doc) => writeln!(
                out,
                "---@field {}{} {} @{}",
                field.lua_name, ro, field.lua_ty, doc
            )
            .unwrap(),
            None => writeln!(out, "---@field {}{} {}", field.lua_name, ro, field.lua_ty).unwrap(),
        }
    }

    // operators — only emit EmmyLua-supported ones

    for mm in &ud.metamethods {
        match mm.as_str() {
            // Display → __tostring via @field
            "Display" => {
                writeln!(out, "---@field __tostring fun(self: {}): string", ud.name).unwrap();
            }
            // PartialOrd → both lt and le
            "PartialOrd" => {
                writeln!(out, "---@operator lt({}): boolean", ud.name).unwrap();
                writeln!(out, "---@operator le({}): boolean", ud.name).unwrap();
            }
            // Other operators
            other => {
                if let Some(op) = trait_to_operator(other, &ud.name) {
                    writeln!(out, "{}", op).unwrap();
                }
            }
        }
    }

    // __close delegate
    for (delegate, _method) in &ud.delegates {
        if delegate == "close" {
            writeln!(out, "---@field __close fun(self: {})", ud.name).unwrap();
        }
    }

    // global variable
    writeln!(out, "{} = {{}}", ud.name).unwrap();
    writeln!(out).unwrap();

    // instance methods
    for method in &ud.methods {
        if method.is_static {
            continue;
        }
        emit_method(out, &ud.name, method);
        writeln!(out).unwrap();
    }

    // static methods
    for method in &ud.methods {
        if !method.is_static {
            continue;
        }
        emit_static_method(out, &ud.name, method);
        writeln!(out).unwrap();
    }
}

fn emit_method(out: &mut String, class_name: &str, method: &MethodInfo) {
    // method doc
    if let Some(ref doc) = method.doc {
        for line in doc.lines() {
            writeln!(out, "--- {}", line).unwrap();
        }
    }
    // params
    for p in &method.params {
        writeln!(out, "---@param {} {}", p.name, p.lua_ty).unwrap();
    }
    // return
    if let Some(ref ret) = method.return_type {
        writeln!(out, "---@return {}", ret).unwrap();
    }

    let params_str: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
    write!(out, "function {}:{}(", class_name, method.lua_name).unwrap();
    write!(out, "{}", params_str.join(", ")).unwrap();
    writeln!(out, ") end").unwrap();
}

fn emit_static_method(out: &mut String, class_name: &str, method: &MethodInfo) {
    // method doc
    if let Some(ref doc) = method.doc {
        for line in doc.lines() {
            writeln!(out, "--- {}", line).unwrap();
        }
    }
    for p in &method.params {
        writeln!(out, "---@param {} {}", p.name, p.lua_ty).unwrap();
    }
    if let Some(ref ret) = method.return_type {
        writeln!(out, "---@return {}", ret).unwrap();
    }

    let params_str: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
    write!(out, "function {}.{}(", class_name, method.lua_name).unwrap();
    write!(out, "{}", params_str.join(", ")).unwrap();
    writeln!(out, ") end").unwrap();
}
