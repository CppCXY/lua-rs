// Simple C declaration parser for FFI

use super::ctype::{CType, CTypeKind};

/// Parse C declarations and return type definitions
pub fn parse_c_declaration(decl: &str) -> Result<Vec<(String, CType)>, String> {
    let mut results = Vec::new();

    // Multi-line parsing for structs
    let cleaned = decl
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join(" ");

    let mut pos = 0;
    while pos < cleaned.len() {
        let remaining = &cleaned[pos..];

        // Parse typedef
        if remaining.starts_with("typedef ") {
            if let Some(end) = remaining.find(';') {
                let typedef_str = &remaining[..=end];
                if let Some((name, ctype)) = parse_typedef(typedef_str)? {
                    results.push((name, ctype));
                }
                pos += end + 1;
                continue;
            }
        }

        // Parse struct definition
        if remaining.starts_with("struct ") {
            if let Some((struct_def, consumed)) = parse_struct_definition(remaining)? {
                results.push(struct_def);
                pos += consumed;
                continue;
            }
        }

        // Parse function declaration
        if let Some(semi) = remaining.find(';') {
            let decl_str = &remaining[..=semi];
            if decl_str.contains('(') && decl_str.contains(')') {
                if let Some((name, ctype)) = parse_function_declaration(decl_str)? {
                    results.push((name, ctype));
                }
                pos += semi + 1;
                continue;
            }
        }

        // Skip unrecognized content
        if let Some(semi) = remaining.find(';') {
            pos += semi + 1;
        } else {
            break;
        }
    }

    Ok(results)
}

fn parse_typedef(line: &str) -> Result<Option<(String, CType)>, String> {
    // typedef <type> <name>;
    let line = line.strip_prefix("typedef ").unwrap().trim();
    let line = line.strip_suffix(';').unwrap_or(line).trim();

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(None);
    }

    let base_type = parts[0];
    let name = parts[parts.len() - 1];

    // Check for pointer
    let is_pointer = parts.iter().any(|p| p.contains('*'));

    let ctype = if is_pointer {
        let elem_type = parse_base_type(base_type)?;
        CType::pointer(elem_type)
    } else {
        parse_base_type(base_type)?
    };

    Ok(Some((name.to_string(), ctype)))
}

fn parse_function_declaration(line: &str) -> Result<Option<(String, CType)>, String> {
    // <return_type> <name>(<params>);
    let line = line.strip_suffix(';').unwrap_or(line).trim();

    let paren_start = line.find('(').ok_or("Invalid function declaration")?;
    let paren_end = line.rfind(')').ok_or("Invalid function declaration")?;

    let signature = &line[..paren_start].trim();
    let params = &line[paren_start + 1..paren_end].trim();

    // Parse return type and function name
    let sig_parts: Vec<&str> = signature.split_whitespace().collect();
    if sig_parts.is_empty() {
        return Ok(None);
    }

    let func_name = sig_parts.last().unwrap();
    let return_type_parts = &sig_parts[..sig_parts.len() - 1];

    let return_type = if return_type_parts.is_empty() {
        CType::new(CTypeKind::Void, 0, 0)
    } else {
        parse_base_type(return_type_parts.join(" ").as_str())?
    };

    // Parse parameters
    let mut param_types = Vec::new();
    let mut is_variadic = false;

    if !params.is_empty() && *params != "void" {
        for param in params.split(',') {
            let param = param.trim();
            if param == "..." {
                is_variadic = true;
                break;
            }

            // Parse parameter type (ignore name)
            // Remove "const" qualifier if present
            let param = param.strip_prefix("const ").unwrap_or(param).trim();

            // Check if it's a pointer type by looking for '*'
            let is_pointer = param.contains('*');

            // Extract the base type (before '*' or name)
            let parts: Vec<&str> = param.split_whitespace().collect();
            if !parts.is_empty() {
                // Get the base type (first part)
                let base_type_str = parts[0];
                // Remove any '*' from it if present
                let base_type_str = base_type_str.trim_end_matches('*');

                let mut param_type = parse_base_type(base_type_str)?;
                // If it's a pointer, wrap in pointer type
                if is_pointer {
                    param_type = CType::pointer(param_type);
                }
                param_types.push(param_type);
            }
        }
    }

    let mut ctype = CType::new(CTypeKind::Function, 8, 8);
    ctype.return_type = Some(Box::new(return_type));
    ctype.param_types = Some(param_types);
    ctype.is_variadic = is_variadic;

    Ok(Some((func_name.to_string(), ctype)))
}

pub fn parse_base_type(type_str: &str) -> Result<CType, String> {
    let type_str = type_str.trim();

    // Handle pointers
    if type_str.ends_with('*') {
        let base = type_str.trim_end_matches('*').trim();
        let elem_type = parse_base_type(base)?;
        return Ok(CType::pointer(elem_type));
    }

    // Handle const qualifier
    let type_str = type_str.strip_prefix("const ").unwrap_or(type_str);

    // Match built-in types
    match type_str {
        "void" => Ok(CType::new(CTypeKind::Void, 0, 0)),
        "char" => Ok(CType::new(CTypeKind::Int8, 1, 1)),
        "signed char" => Ok(CType::new(CTypeKind::Int8, 1, 1)),
        "unsigned char" => Ok(CType::new(CTypeKind::UInt8, 1, 1)),
        "short" | "short int" => Ok(CType::new(CTypeKind::Int16, 2, 2)),
        "unsigned short" | "unsigned short int" => Ok(CType::new(CTypeKind::UInt16, 2, 2)),
        "int" => Ok(CType::new(CTypeKind::Int32, 4, 4)),
        "unsigned int" | "unsigned" => Ok(CType::new(CTypeKind::UInt32, 4, 4)),
        "long" | "long int" => Ok(CType::new(CTypeKind::Int64, 8, 8)),
        "unsigned long" | "unsigned long int" => Ok(CType::new(CTypeKind::UInt64, 8, 8)),
        "long long" | "long long int" => Ok(CType::new(CTypeKind::Int64, 8, 8)),
        "unsigned long long" => Ok(CType::new(CTypeKind::UInt64, 8, 8)),
        "float" => Ok(CType::new(CTypeKind::Float, 4, 4)),
        "double" => Ok(CType::new(CTypeKind::Double, 8, 8)),
        "bool" | "_Bool" => Ok(CType::new(CTypeKind::Bool, 1, 1)),
        "size_t" => Ok(CType::new(CTypeKind::UInt64, 8, 8)),
        "ssize_t" => Ok(CType::new(CTypeKind::Int64, 8, 8)),
        "intptr_t" => Ok(CType::new(CTypeKind::Int64, 8, 8)),
        "uintptr_t" => Ok(CType::new(CTypeKind::UInt64, 8, 8)),
        "int8_t" => Ok(CType::new(CTypeKind::Int8, 1, 1)),
        "uint8_t" => Ok(CType::new(CTypeKind::UInt8, 1, 1)),
        "int16_t" => Ok(CType::new(CTypeKind::Int16, 2, 2)),
        "uint16_t" => Ok(CType::new(CTypeKind::UInt16, 2, 2)),
        "int32_t" => Ok(CType::new(CTypeKind::Int32, 4, 4)),
        "uint32_t" => Ok(CType::new(CTypeKind::UInt32, 4, 4)),
        "int64_t" => Ok(CType::new(CTypeKind::Int64, 8, 8)),
        "uint64_t" => Ok(CType::new(CTypeKind::UInt64, 8, 8)),
        _ => Err(format!("Unknown type: {}", type_str)),
    }
}

/// Parse struct definition
/// Returns (name, ctype) and number of characters consumed
fn parse_struct_definition(input: &str) -> Result<Option<((String, CType), usize)>, String> {
    use super::ctype::StructField;
    use std::collections::HashMap;

    // struct Name { ... };
    if !input.starts_with("struct ") {
        return Ok(None);
    }

    let input = input.trim_start_matches("struct ").trim();

    // Find struct name
    let name_end = input
        .find(|c: char| c.is_whitespace() || c == '{')
        .ok_or("Invalid struct definition")?;
    let struct_name = input[..name_end].trim().to_string();

    let remaining = input[name_end..].trim();

    // Find the opening brace
    if !remaining.starts_with('{') {
        return Ok(None);
    }

    // Find matching closing brace
    let mut brace_count = 0;
    let mut end_pos = 0;
    for (i, ch) in remaining.chars().enumerate() {
        if ch == '{' {
            brace_count += 1;
        } else if ch == '}' {
            brace_count -= 1;
            if brace_count == 0 {
                end_pos = i;
                break;
            }
        }
    }

    if end_pos == 0 {
        return Err("Unclosed struct definition".to_string());
    }

    let body = &remaining[1..end_pos].trim();

    // Parse fields
    let mut fields = HashMap::new();
    let mut current_offset = 0;

    for field_decl in body.split(';') {
        let field_decl = field_decl.trim();
        if field_decl.is_empty() {
            continue;
        }

        // Parse field: type name
        let parts: Vec<&str> = field_decl.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        // Handle pointer types
        let is_pointer = field_decl.contains('*');
        let type_part = parts[0].trim_end_matches('*');
        let field_name = parts.last().unwrap().trim_start_matches('*');

        let mut field_type = parse_base_type(type_part)?;
        if is_pointer {
            field_type = CType::pointer(field_type);
        }

        // Calculate alignment and offset
        let field_align = field_type.alignment;
        let aligned_offset = (current_offset + field_align - 1) / field_align * field_align;

        fields.insert(
            field_name.to_string(),
            StructField {
                ctype: field_type.clone(),
                offset: aligned_offset,
            },
        );

        current_offset = aligned_offset + field_type.size;
    }

    // Calculate total size with final padding
    let max_align = fields
        .values()
        .map(|f| f.ctype.alignment)
        .max()
        .unwrap_or(1);
    let total_size = (current_offset + max_align - 1) / max_align * max_align;

    let mut struct_type = CType::new(CTypeKind::Struct, total_size, max_align);
    struct_type.name = Some(struct_name.clone());
    struct_type.fields = Some(fields);

    // Calculate total consumed characters
    let consumed = "struct ".len() + name_end + end_pos + 2; // +2 for closing } and ;

    Ok(Some(((struct_name, struct_type), consumed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_typedef() {
        let decl = "typedef int myint;";
        let result = parse_c_declaration(decl).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "myint");
        assert_eq!(result[0].1.kind, CTypeKind::Int32);
    }

    #[test]
    fn test_parse_function() {
        let decl = "int add(int a, int b);";
        let result = parse_c_declaration(decl).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "add");
        assert_eq!(result[0].1.kind, CTypeKind::Function);
    }

    #[test]
    fn test_parse_pointer() {
        let decl = "typedef char* string;";
        let result = parse_c_declaration(decl).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "string");
        assert_eq!(result[0].1.kind, CTypeKind::Pointer);
    }
}
