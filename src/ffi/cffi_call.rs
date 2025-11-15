// C function invocation support
// Provides mechanism to call C functions from Lua through FFI

use crate::ffi::ctype::{CType, CTypeKind};
use crate::lua_value::LuaValue;
use std::ffi::CString;

/// Represents a C function call
pub struct CFunctionCall {
    pub ptr: *mut u8,
    pub return_type: CType,
    pub param_types: Vec<CType>,
}

impl CFunctionCall {
    pub fn new(ptr: *mut u8, return_type: CType, param_types: Vec<CType>) -> Self {
        CFunctionCall {
            ptr,
            return_type,
            param_types,
        }
    }

    /// Call the C function with given Lua arguments
    /// SAFETY: This is highly unsafe and simplified
    pub fn call(&self, args: &[LuaValue]) -> Result<LuaValue, String> {
        // Verify argument count
        if args.len() != self.param_types.len() {
            return Err(format!(
                "wrong number of arguments (expected {}, got {})",
                self.param_types.len(),
                args.len()
            ));
        }

        // For now, implement only simple calling conventions for common signatures
        // Full implementation would require libffi or manual assembly

        unsafe {
            match (self.return_type.kind, self.param_types.as_slice()) {
                // void function(void)
                (CTypeKind::Void, []) => {
                    type FnType = unsafe extern "C" fn();
                    let f: FnType = std::mem::transmute(self.ptr);
                    f();
                    Ok(LuaValue::nil())
                }

                // int function(void)
                (CTypeKind::Int32, []) => {
                    type FnType = unsafe extern "C" fn() -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f();
                    Ok(LuaValue::integer(result as i64))
                }

                // int function(const char*)
                (CTypeKind::Int32, [param]) if matches!(param.kind, CTypeKind::Pointer) => {
                    let arg_str = args[0].as_string_ptr()
                        .ok_or("expected string argument")?;
                    let c_str = CString::new((*arg_str).as_str())
                        .map_err(|_| "invalid string")?;

                    type FnType = unsafe extern "C" fn(*const i8) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(c_str.as_ptr());
                    Ok(LuaValue::integer(result as i64))
                }

                // int function(int)
                (CTypeKind::Int32, [param]) if matches!(param.kind, CTypeKind::Int32) => {
                    let arg_int = args[0].as_integer()
                        .ok_or("expected integer argument")?;

                    type FnType = unsafe extern "C" fn(i32) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(arg_int as i32);
                    Ok(LuaValue::integer(result as i64))
                }

                // int function(int, int)
                (CTypeKind::Int32, [p1, p2])
                    if matches!(p1.kind, CTypeKind::Int32) && matches!(p2.kind, CTypeKind::Int32) =>
                {
                    let arg1 = args[0].as_integer().ok_or("expected integer argument 1")?;
                    let arg2 = args[1].as_integer().ok_or("expected integer argument 2")?;

                    type FnType = unsafe extern "C" fn(i32, i32) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(arg1 as i32, arg2 as i32);
                    Ok(LuaValue::integer(result as i64))
                }

                // void* function(size_t) - malloc-like
                (CTypeKind::Pointer, [param]) if matches!(param.kind, CTypeKind::UInt64 | CTypeKind::UInt32) => {
                    let size = args[0].as_integer()
                        .ok_or("expected size argument")?;

                    type FnType = unsafe extern "C" fn(usize) -> *mut u8;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(size as usize);
                    Ok(LuaValue::integer(result as i64))
                }

                // void function(void*) - free-like
                (CTypeKind::Void, [param]) if matches!(param.kind, CTypeKind::Pointer) => {
                    let ptr = args[0].as_integer()
                        .ok_or("expected pointer argument")?;

                    type FnType = unsafe extern "C" fn(*mut u8);
                    let f: FnType = std::mem::transmute(self.ptr);
                    f(ptr as *mut u8);
                    Ok(LuaValue::nil())
                }

                _ => {
                    Err(format!(
                        "unsupported C function signature (return: {:?}, params: {} args)",
                        self.return_type.kind,
                        self.param_types.len()
                    ))
                }
            }
        }
    }
}
