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
                    let arg_str = args[0].as_string_ptr().ok_or("expected string argument")?;
                    let c_str = CString::new((*arg_str).as_str()).map_err(|_| "invalid string")?;

                    type FnType = unsafe extern "C" fn(*const i8) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(c_str.as_ptr());
                    Ok(LuaValue::integer(result as i64))
                }

                // int function(int)
                (CTypeKind::Int32, [param]) if matches!(param.kind, CTypeKind::Int32) => {
                    let arg_int = args[0].as_integer().ok_or("expected integer argument")?;

                    type FnType = unsafe extern "C" fn(i32) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(arg_int as i32);
                    Ok(LuaValue::integer(result as i64))
                }

                // int function(int, int)
                (CTypeKind::Int32, [p1, p2])
                    if matches!(p1.kind, CTypeKind::Int32)
                        && matches!(p2.kind, CTypeKind::Int32) =>
                {
                    let arg1 = args[0].as_integer().ok_or("expected integer argument 1")?;
                    let arg2 = args[1].as_integer().ok_or("expected integer argument 2")?;

                    type FnType = unsafe extern "C" fn(i32, i32) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(arg1 as i32, arg2 as i32);
                    Ok(LuaValue::integer(result as i64))
                }

                // void* function(size_t) - malloc-like
                (CTypeKind::Pointer, [param])
                    if matches!(param.kind, CTypeKind::UInt64 | CTypeKind::UInt32) =>
                {
                    let size = args[0].as_integer().ok_or("expected size argument")?;

                    type FnType = unsafe extern "C" fn(usize) -> *mut u8;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(size as usize);
                    Ok(LuaValue::integer(result as i64))
                }

                // void* function(size_t, size_t) - calloc-like
                (CTypeKind::Pointer, [p1, p2])
                    if matches!(p1.kind, CTypeKind::UInt64 | CTypeKind::UInt32)
                        && matches!(p2.kind, CTypeKind::UInt64 | CTypeKind::UInt32) =>
                {
                    let nmemb = args[0].as_integer().ok_or("expected size argument 1")?;
                    let size = args[1].as_integer().ok_or("expected size argument 2")?;

                    type FnType = unsafe extern "C" fn(usize, usize) -> *mut u8;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(nmemb as usize, size as usize);
                    Ok(LuaValue::integer(result as i64))
                }

                // void* function(void*, size_t) - realloc-like
                (CTypeKind::Pointer, [p1, p2])
                    if matches!(p1.kind, CTypeKind::Pointer)
                        && matches!(p2.kind, CTypeKind::UInt64 | CTypeKind::UInt32) =>
                {
                    let ptr = args[0].as_integer().ok_or("expected pointer argument")?;
                    let size = args[1].as_integer().ok_or("expected size argument")?;

                    type FnType = unsafe extern "C" fn(*mut u8, usize) -> *mut u8;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(ptr as *mut u8, size as usize);
                    Ok(LuaValue::integer(result as i64))
                }

                // void function(void*) - free-like
                (CTypeKind::Void, [param]) if matches!(param.kind, CTypeKind::Pointer) => {
                    let ptr = args[0].as_integer().ok_or("expected pointer argument")?;

                    type FnType = unsafe extern "C" fn(*mut u8);
                    let f: FnType = std::mem::transmute(self.ptr);
                    f(ptr as *mut u8);
                    Ok(LuaValue::nil())
                }

                // int function(const char*, const char*) - strcmp-like
                (CTypeKind::Int32, [p1, p2])
                    if matches!(p1.kind, CTypeKind::Pointer)
                        && matches!(p2.kind, CTypeKind::Pointer) =>
                {
                    let arg1_str = args[0]
                        .as_string_ptr()
                        .ok_or("expected string argument 1")?;
                    let arg2_str = args[1]
                        .as_string_ptr()
                        .ok_or("expected string argument 2")?;
                    let c_str1 =
                        CString::new((*arg1_str).as_str()).map_err(|_| "invalid string 1")?;
                    let c_str2 =
                        CString::new((*arg2_str).as_str()).map_err(|_| "invalid string 2")?;

                    type FnType = unsafe extern "C" fn(*const i8, *const i8) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(c_str1.as_ptr(), c_str2.as_ptr());
                    Ok(LuaValue::integer(result as i64))
                }

                // double function(double) - sqrt, sin, cos
                (CTypeKind::Double, [param]) if matches!(param.kind, CTypeKind::Double) => {
                    let arg = args[0].as_number().ok_or("expected number argument")?;

                    type FnType = unsafe extern "C" fn(f64) -> f64;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(arg);
                    Ok(LuaValue::number(result))
                }

                // double function(double, double) - pow, atan2
                (CTypeKind::Double, [p1, p2])
                    if matches!(p1.kind, CTypeKind::Double)
                        && matches!(p2.kind, CTypeKind::Double) =>
                {
                    let arg1 = args[0].as_number().ok_or("expected number argument 1")?;
                    let arg2 = args[1].as_number().ok_or("expected number argument 2")?;

                    type FnType = unsafe extern "C" fn(f64, f64) -> f64;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(arg1, arg2);
                    Ok(LuaValue::number(result))
                }

                // void* function(void*, int, size_t) - memset-like
                (CTypeKind::Pointer, [p1, p2, p3])
                    if matches!(p1.kind, CTypeKind::Pointer)
                        && matches!(p2.kind, CTypeKind::Int32)
                        && matches!(p3.kind, CTypeKind::UInt64 | CTypeKind::UInt32) =>
                {
                    let ptr = args[0].as_integer().ok_or("expected pointer argument")?;
                    let value = args[1].as_integer().ok_or("expected int argument")?;
                    let size = args[2].as_integer().ok_or("expected size argument")?;

                    type FnType = unsafe extern "C" fn(*mut u8, i32, usize) -> *mut u8;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(ptr as *mut u8, value as i32, size as usize);
                    Ok(LuaValue::integer(result as i64))
                }

                // void* function(void*, const void*, size_t) - memcpy-like
                (CTypeKind::Pointer, [p1, p2, p3])
                    if matches!(p1.kind, CTypeKind::Pointer)
                        && matches!(p2.kind, CTypeKind::Pointer)
                        && matches!(p3.kind, CTypeKind::UInt64 | CTypeKind::UInt32) =>
                {
                    let dst = args[0].as_integer().ok_or("expected pointer argument 1")?;
                    let src = args[1].as_integer().ok_or("expected pointer argument 2")?;
                    let size = args[2].as_integer().ok_or("expected size argument")?;

                    type FnType = unsafe extern "C" fn(*mut u8, *const u8, usize) -> *mut u8;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(dst as *mut u8, src as *const u8, size as usize);
                    Ok(LuaValue::integer(result as i64))
                }

                // long function(void) - time-like
                (CTypeKind::Int64, []) => {
                    type FnType = unsafe extern "C" fn() -> i64;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f();
                    Ok(LuaValue::integer(result))
                }

                // int function(int, const char*) - mixed params
                (CTypeKind::Int32, [p1, p2])
                    if matches!(p1.kind, CTypeKind::Int32)
                        && matches!(p2.kind, CTypeKind::Pointer) =>
                {
                    let arg1 = args[0].as_integer().ok_or("expected integer argument 1")?;
                    let arg2_str = args[1]
                        .as_string_ptr()
                        .ok_or("expected string argument 2")?;
                    let c_str = CString::new((*arg2_str).as_str()).map_err(|_| "invalid string")?;

                    type FnType = unsafe extern "C" fn(i32, *const i8) -> i32;
                    let f: FnType = std::mem::transmute(self.ptr);
                    let result = f(arg1 as i32, c_str.as_ptr());
                    Ok(LuaValue::integer(result as i64))
                }

                _ => Err(format!(
                    "unsupported C function signature (return: {:?}, params: {} args)",
                    self.return_type.kind,
                    self.param_types.len()
                )),
            }
        }
    }
}
