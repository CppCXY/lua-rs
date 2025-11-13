// JIT-friendly value representation
// Uses a fixed-layout tagged union that JIT compiled code can directly manipulate

use std::mem;

/// JIT value type tags (8-bit discriminant)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitValueTag {
    Nil = 0,
    Boolean = 1,
    Integer = 2,
    Float = 3,
    // For complex types, we still need to go through the interpreter
}

/// JIT-compatible value representation
/// Layout: 16 bytes total
/// - 8 bytes: tag (u64, only low byte used)
/// - 8 bytes: data (union of i64/f64/bool)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JitValue {
    tag: u64,  // Only use low byte, rest reserved for future use
    data: JitValueData,
}

#[repr(C)]
#[derive(Clone, Copy)]
union JitValueData {
    integer: i64,
    float: f64,
    boolean: bool,
}

impl JitValue {
    pub const SIZE: usize = 16;
    
    pub fn nil() -> Self {
        JitValue {
            tag: JitValueTag::Nil as u64,
            data: JitValueData { integer: 0 },
        }
    }
    
    pub fn boolean(b: bool) -> Self {
        JitValue {
            tag: JitValueTag::Boolean as u64,
            data: JitValueData { boolean: b },
        }
    }
    
    pub fn integer(i: i64) -> Self {
        JitValue {
            tag: JitValueTag::Integer as u64,
            data: JitValueData { integer: i },
        }
    }
    
    pub fn float(f: f64) -> Self {
        JitValue {
            tag: JitValueTag::Float as u64,
            data: JitValueData { float: f },
        }
    }
    
    pub fn tag(&self) -> u8 {
        self.tag as u8
    }
    
    pub fn as_integer(&self) -> Option<i64> {
        if self.tag() == JitValueTag::Integer as u8 {
            Some(unsafe { self.data.integer })
        } else {
            None
        }
    }
    
    pub fn as_float(&self) -> Option<f64> {
        if self.tag() == JitValueTag::Float as u8 {
            Some(unsafe { self.data.float })
        } else {
            None
        }
    }
    
    pub fn as_boolean(&self) -> Option<bool> {
        if self.tag() == JitValueTag::Boolean as u8 {
            Some(unsafe { self.data.boolean })
        } else {
            None
        }
    }
    
    pub fn is_nil(&self) -> bool {
        self.tag() == JitValueTag::Nil as u8
    }
}

impl std::fmt::Debug for JitValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tag() {
            t if t == JitValueTag::Nil as u8 => write!(f, "nil"),
            t if t == JitValueTag::Boolean as u8 => write!(f, "{}", unsafe { self.data.boolean }),
            t if t == JitValueTag::Integer as u8 => write!(f, "{}", unsafe { self.data.integer }),
            t if t == JitValueTag::Float as u8 => write!(f, "{}", unsafe { self.data.float }),
            _ => write!(f, "invalid({} )", self.tag),
        }
    }
}

// Verify size at compile time
const _: () = assert!(mem::size_of::<JitValue>() == 16);
const _: () = assert!(mem::align_of::<JitValue>() == 8);

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_jit_value_size() {
        assert_eq!(mem::size_of::<JitValue>(), 16);
        assert_eq!(mem::align_of::<JitValue>(), 8);
    }
    
    #[test]
    fn test_jit_value_creation() {
        let nil = JitValue::nil();
        assert!(nil.is_nil());
        
        let int_val = JitValue::integer(42);
        assert_eq!(int_val.as_integer(), Some(42));
        
        let float_val = JitValue::float(3.14);
        assert_eq!(float_val.as_float(), Some(3.14));
        
        let bool_val = JitValue::boolean(true);
        assert_eq!(bool_val.as_boolean(), Some(true));
    }
}
