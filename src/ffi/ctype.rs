// C type system for FFI

use crate::lua_value::LuaValue;
use std::collections::HashMap;

/// C type kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CTypeKind {
    Void,
    Bool,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float,
    Double,
    Pointer,
    Array,
    Struct,
    Union,
    Function,
    Enum,
}

/// C type definition
#[derive(Debug, Clone)]
pub struct CType {
    pub kind: CTypeKind,
    pub size: usize,
    pub alignment: usize,
    pub name: Option<String>,
    
    // For pointer/array types
    pub element_type: Option<Box<CType>>,
    pub array_size: Option<usize>,
    
    // For struct/union types
    pub fields: Option<HashMap<String, StructField>>,
    
    // For function types
    pub return_type: Option<Box<CType>>,
    pub param_types: Option<Vec<CType>>,
    pub is_variadic: bool,
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub ctype: CType,
    pub offset: usize,
}

impl CType {
    pub fn new(kind: CTypeKind, size: usize, alignment: usize) -> Self {
        CType {
            kind,
            size,
            alignment,
            name: None,
            element_type: None,
            array_size: None,
            fields: None,
            return_type: None,
            param_types: None,
            is_variadic: false,
        }
    }

    pub fn pointer(element_type: CType) -> Self {
        CType {
            kind: CTypeKind::Pointer,
            size: 8, // 64-bit pointer
            alignment: 8,
            name: None,
            element_type: Some(Box::new(element_type)),
            array_size: None,
            fields: None,
            return_type: None,
            param_types: None,
            is_variadic: false,
        }
    }

    pub fn array(element_type: CType, size: usize) -> Self {
        let elem_size = element_type.size;
        let elem_align = element_type.alignment;
        CType {
            kind: CTypeKind::Array,
            size: elem_size * size,
            alignment: elem_align,
            name: None,
            element_type: Some(Box::new(element_type)),
            array_size: Some(size),
            fields: None,
            return_type: None,
            param_types: None,
            is_variadic: false,
        }
    }

    pub fn structure(name: String, fields: HashMap<String, StructField>) -> Self {
        // Calculate struct size and alignment
        let mut size = 0;
        let mut alignment = 1;
        
        for field in fields.values() {
            alignment = alignment.max(field.ctype.alignment);
            size = size.max(field.offset + field.ctype.size);
        }
        
        // Add padding at the end
        size = (size + alignment - 1) & !(alignment - 1);
        
        CType {
            kind: CTypeKind::Struct,
            size,
            alignment,
            name: Some(name),
            element_type: None,
            array_size: None,
            fields: Some(fields),
            return_type: None,
            param_types: None,
            is_variadic: false,
        }
    }

    pub fn get_field_offset(&self, field_name: &str) -> Result<usize, String> {
        if let Some(fields) = &self.fields {
            fields.get(field_name)
                .map(|f| f.offset)
                .ok_or_else(|| format!("Field '{}' not found", field_name))
        } else {
            Err("Not a struct or union type".to_string())
        }
    }
}

/// C data object - wraps actual C data
#[derive(Debug, Clone)]
pub struct CData {
    pub ctype: CType,
    pub data: Vec<u8>,
    pub is_pointer: bool,
    pub pointer_value: usize,
}

impl CData {
    pub fn new(ctype: CType) -> Self {
        let size = ctype.size;
        CData {
            ctype,
            data: vec![0; size],
            is_pointer: false,
            pointer_value: 0,
        }
    }

    pub fn new_pointer(ptr: usize) -> Self {
        CData {
            ctype: CType::new(CTypeKind::Pointer, 8, 8),
            data: ptr.to_ne_bytes().to_vec(),
            is_pointer: true,
            pointer_value: ptr,
        }
    }

    pub fn from_lua_value(ctype: CType, value: LuaValue) -> Result<Self, String> {
        let mut cdata = CData::new(ctype.clone());
        
        match ctype.kind {
            CTypeKind::Int8 | CTypeKind::Int16 | CTypeKind::Int32 | CTypeKind::Int64 => {
                if let Some(i) = value.as_integer() {
                    cdata.write_integer(i);
                } else {
                    return Err("Expected integer value".to_string());
                }
            }
            CTypeKind::UInt8 | CTypeKind::UInt16 | CTypeKind::UInt32 | CTypeKind::UInt64 => {
                if let Some(i) = value.as_integer() {
                    cdata.write_integer(i);
                } else {
                    return Err("Expected integer value".to_string());
                }
            }
            CTypeKind::Float => {
                if let Some(f) = value.as_number() {
                    cdata.write_float(f as f32);
                } else {
                    return Err("Expected number value".to_string());
                }
            }
            CTypeKind::Double => {
                if let Some(f) = value.as_number() {
                    cdata.write_double(f);
                } else {
                    return Err("Expected number value".to_string());
                }
            }
            CTypeKind::Bool => {
                if let Some(b) = value.as_boolean() {
                    cdata.data[0] = if b { 1 } else { 0 };
                } else {
                    return Err("Expected boolean value".to_string());
                }
            }
            CTypeKind::Pointer => {
                if let Some(i) = value.as_integer() {
                    cdata.pointer_value = i as usize;
                    cdata.data = i.to_ne_bytes().to_vec();
                } else {
                    return Err("Expected integer or pointer value".to_string());
                }
            }
            _ => {
                return Err(format!("Unsupported conversion for type {:?}", ctype.kind));
            }
        }
        
        Ok(cdata)
    }

    pub fn to_lua_value(&self) -> LuaValue {
        match self.ctype.kind {
            CTypeKind::Int8 => LuaValue::integer(self.read_i8() as i64),
            CTypeKind::UInt8 => LuaValue::integer(self.read_u8() as i64),
            CTypeKind::Int16 => LuaValue::integer(self.read_i16() as i64),
            CTypeKind::UInt16 => LuaValue::integer(self.read_u16() as i64),
            CTypeKind::Int32 => LuaValue::integer(self.read_i32() as i64),
            CTypeKind::UInt32 => LuaValue::integer(self.read_u32() as i64),
            CTypeKind::Int64 => LuaValue::integer(self.read_i64()),
            CTypeKind::UInt64 => LuaValue::integer(self.read_u64() as i64),
            CTypeKind::Float => LuaValue::float(self.read_f32() as f64),
            CTypeKind::Double => LuaValue::float(self.read_f64()),
            CTypeKind::Bool => LuaValue::boolean(self.data[0] != 0),
            CTypeKind::Pointer => LuaValue::integer(self.pointer_value as i64),
            _ => LuaValue::nil(),
        }
    }

    pub fn as_pointer(&self) -> Result<*mut u8, String> {
        if self.is_pointer || self.ctype.kind == CTypeKind::Pointer {
            Ok(self.pointer_value as *mut u8)
        } else {
            // Return pointer to data
            Ok(self.data.as_ptr() as *mut u8)
        }
    }

    // Write methods
    fn write_integer(&mut self, value: i64) {
        match self.ctype.size {
            1 => self.data[0] = value as u8,
            2 => self.data[0..2].copy_from_slice(&(value as i16).to_ne_bytes()),
            4 => self.data[0..4].copy_from_slice(&(value as i32).to_ne_bytes()),
            8 => self.data[0..8].copy_from_slice(&value.to_ne_bytes()),
            _ => {}
        }
    }

    fn write_float(&mut self, value: f32) {
        self.data[0..4].copy_from_slice(&value.to_ne_bytes());
    }

    fn write_double(&mut self, value: f64) {
        self.data[0..8].copy_from_slice(&value.to_ne_bytes());
    }

    // Read methods
    fn read_i8(&self) -> i8 {
        self.data[0] as i8
    }

    fn read_u8(&self) -> u8 {
        self.data[0]
    }

    fn read_i16(&self) -> i16 {
        i16::from_ne_bytes([self.data[0], self.data[1]])
    }

    fn read_u16(&self) -> u16 {
        u16::from_ne_bytes([self.data[0], self.data[1]])
    }

    fn read_i32(&self) -> i32 {
        i32::from_ne_bytes([self.data[0], self.data[1], self.data[2], self.data[3]])
    }

    fn read_u32(&self) -> u32 {
        u32::from_ne_bytes([self.data[0], self.data[1], self.data[2], self.data[3]])
    }

    fn read_i64(&self) -> i64 {
        i64::from_ne_bytes([
            self.data[0], self.data[1], self.data[2], self.data[3],
            self.data[4], self.data[5], self.data[6], self.data[7],
        ])
    }

    fn read_u64(&self) -> u64 {
        u64::from_ne_bytes([
            self.data[0], self.data[1], self.data[2], self.data[3],
            self.data[4], self.data[5], self.data[6], self.data[7],
        ])
    }

    fn read_f32(&self) -> f32 {
        f32::from_ne_bytes([self.data[0], self.data[1], self.data[2], self.data[3]])
    }

    fn read_f64(&self) -> f64 {
        f64::from_ne_bytes([
            self.data[0], self.data[1], self.data[2], self.data[3],
            self.data[4], self.data[5], self.data[6], self.data[7],
        ])
    }
}
