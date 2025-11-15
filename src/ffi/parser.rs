// Simple C declaration parser for FFI

use super::ctype::{CType, CTypeKind};

/// Parse C declarations and return type definitions
pub fn parse_c_declaration(decl: &str) -> Result<Vec<(String, CType)>, String> {
    let mut results = Vec::new();
    
    // Simple line-by-line parsing
    for line in decl.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        
        // Parse typedef
        if line.starts_with("typedef ") {
            let typedef = parse_typedef(line)?;
            if let Some((name, ctype)) = typedef {
                results.push((name, ctype));
            }
            continue;
        }
        
        // Parse struct definition
        if line.starts_with("struct ") {
            // For now, skip - need multiline parsing
            continue;
        }
        
        // Parse function declaration
        if line.contains('(') && line.contains(')') && line.ends_with(';') {
            let func_decl = parse_function_declaration(line)?;
            if let Some((name, ctype)) = func_decl {
                results.push((name, ctype));
            }
            continue;
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
