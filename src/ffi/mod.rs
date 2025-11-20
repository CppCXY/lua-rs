// FFI (Foreign Function Interface) implementation
// Compatible with LuaJIT FFI API

use crate::lib_registry;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaError, LuaResult, LuaVM};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod callback;
mod cffi_call;
mod ctype;
mod parser;

pub use callback::CCallback;
pub use cffi_call::CFunctionCall;
pub use ctype::{CData, CType, CTypeKind};
pub use parser::parse_c_declaration;

/// Function signature information
#[derive(Clone)]
pub struct CFunctionSignature {
    pub return_type: CType,
    pub param_types: Vec<CType>,
}

/// FFI state - manages C types, libraries, and callbacks
pub struct FFIState {
    /// Loaded C libraries
    libraries: HashMap<String, libloading::Library>,
    /// Registered C types
    types: HashMap<String, CType>,
    /// Type aliases
    type_aliases: HashMap<String, String>,
    /// C function signatures
    function_signatures: HashMap<String, CFunctionSignature>,
    /// C callbacks
    #[allow(unused)]
    callbacks: Vec<Arc<Mutex<CCallback>>>,
}

impl FFIState {
    pub fn new() -> Self {
        let mut state = FFIState {
            libraries: HashMap::new(),
            types: HashMap::new(),
            type_aliases: HashMap::new(),
            function_signatures: HashMap::new(),
            callbacks: Vec::new(),
        };

        // Register built-in C types
        state.register_builtin_types();
        state
    }

    fn register_builtin_types(&mut self) {
        use CTypeKind::*;

        // Integer types
        self.types
            .insert("void".to_string(), CType::new(Void, 0, 0));
        self.types
            .insert("char".to_string(), CType::new(Int8, 1, 1));
        self.types
            .insert("signed char".to_string(), CType::new(Int8, 1, 1));
        self.types
            .insert("unsigned char".to_string(), CType::new(UInt8, 1, 1));
        self.types
            .insert("short".to_string(), CType::new(Int16, 2, 2));
        self.types
            .insert("unsigned short".to_string(), CType::new(UInt16, 2, 2));
        self.types
            .insert("int".to_string(), CType::new(Int32, 4, 4));
        self.types
            .insert("unsigned int".to_string(), CType::new(UInt32, 4, 4));
        self.types
            .insert("long".to_string(), CType::new(Int64, 8, 8));
        self.types
            .insert("unsigned long".to_string(), CType::new(UInt64, 8, 8));
        self.types
            .insert("long long".to_string(), CType::new(Int64, 8, 8));
        self.types
            .insert("unsigned long long".to_string(), CType::new(UInt64, 8, 8));

        // Floating point types
        self.types
            .insert("float".to_string(), CType::new(Float, 4, 4));
        self.types
            .insert("double".to_string(), CType::new(Double, 8, 8));

        // Size types
        self.types
            .insert("size_t".to_string(), CType::new(UInt64, 8, 8));
        self.types
            .insert("ssize_t".to_string(), CType::new(Int64, 8, 8));
        self.types
            .insert("intptr_t".to_string(), CType::new(Int64, 8, 8));
        self.types
            .insert("uintptr_t".to_string(), CType::new(UInt64, 8, 8));

        // Boolean
        self.types
            .insert("bool".to_string(), CType::new(Bool, 1, 1));

        // Stdint types
        self.types
            .insert("int8_t".to_string(), CType::new(Int8, 1, 1));
        self.types
            .insert("uint8_t".to_string(), CType::new(UInt8, 1, 1));
        self.types
            .insert("int16_t".to_string(), CType::new(Int16, 2, 2));
        self.types
            .insert("uint16_t".to_string(), CType::new(UInt16, 2, 2));
        self.types
            .insert("int32_t".to_string(), CType::new(Int32, 4, 4));
        self.types
            .insert("uint32_t".to_string(), CType::new(UInt32, 4, 4));
        self.types
            .insert("int64_t".to_string(), CType::new(Int64, 8, 8));
        self.types
            .insert("uint64_t".to_string(), CType::new(UInt64, 8, 8));
    }

    pub fn load_library(&mut self, name: &str) -> LuaResult<()> {
        if self.libraries.contains_key(name) {
            return Ok(());
        }

        let lib = unsafe {
            libloading::Library::new(name).map_err(|e| {
                LuaError::RuntimeError(format!("Failed to load library '{}': {}", name, e))
            })?
        };

        self.libraries.insert(name.to_string(), lib);
        Ok(())
    }

    pub fn get_symbol(&self, lib_name: &str, symbol: &str) -> LuaResult<*mut u8> {
        let lib = self
            .libraries
            .get(lib_name)
            .ok_or_else(|| LuaError::RuntimeError(format!("Library '{}' not loaded", lib_name)))?;
        unsafe {
            let symbol_ptr: libloading::Symbol<*mut u8> =
                lib.get(symbol.as_bytes()).map_err(|e| {
                    LuaError::RuntimeError(format!("Symbol '{}' not found: {}", symbol, e))
                })?;
            Ok(*symbol_ptr)
        }
    }

    pub fn register_type(&mut self, name: String, ctype: CType) {
        self.types.insert(name, ctype);
    }

    pub fn register_function(&mut self, name: String, return_type: CType, param_types: Vec<CType>) {
        self.function_signatures.insert(
            name,
            CFunctionSignature {
                return_type,
                param_types,
            },
        );
    }

    pub fn get_function_signature(&self, name: &str) -> Option<&CFunctionSignature> {
        self.function_signatures.get(name)
    }

    pub fn get_type(&self, name: &str) -> Option<&CType> {
        // Check direct type
        if let Some(t) = self.types.get(name) {
            return Some(t);
        }

        // Check alias
        if let Some(alias) = self.type_aliases.get(name) {
            return self.types.get(alias);
        }

        None
    }
}

// FFI library functions for Lua

/// ffi.cdef(def) - Parse C declarations
pub fn ffi_cdef(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let def = lib_registry::get_string(vm, 0, "ffi.cdef")?;

    // Parse C declarations
    let declarations = match parse_c_declaration(&def) {
        Ok(decls) => decls,
        Err(e) => return Err(LuaError::RuntimeError(format!("ffi.cdef error: {}", e))),
    };

    // Register types and functions
    let ffi_state = vm.get_ffi_state_mut();
    for (name, ctype) in declarations {
        // Check if this is a function type
        if matches!(ctype.kind, CTypeKind::Function) {
            // Extract return type and parameters
            if let Some(return_type) = &ctype.return_type {
                let param_types = ctype
                    .param_types
                    .as_ref()
                    .map(|v| v.clone())
                    .unwrap_or_default();
                ffi_state.register_function(name.clone(), (**return_type).clone(), param_types);
            }
        }
        // Also register the ctype itself
        ffi_state.register_type(name, ctype);
    }

    Ok(MultiValue::empty())
}

/// ffi.C - Access to C namespace (standard library)
pub fn ffi_c_index(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let name = crate::lib_registry::get_string(vm, 1, "ffi.C.__index")?;

    // Try to get symbol from C standard library
    #[cfg(target_os = "windows")]
    let lib_name = "msvcrt.dll";
    #[cfg(target_os = "linux")]
    let lib_name = "libc.so.6";
    #[cfg(target_os = "macos")]
    let lib_name = "libSystem.dylib";

    // Ensure library is loaded
    let ffi_state = vm.get_ffi_state_mut();
    ffi_state.load_library(lib_name)?;

    let symbol_ptr = ffi_state.get_symbol(lib_name, &name)?;

    // Get function signature if available
    let signature = ffi_state.get_function_signature(&name).cloned();

    // Create a table to wrap the function pointer
    // Table fields: _ptr (the function pointer), _name (function name)
    // With __call metamethod for invoking the function
    let wrapper = vm.create_table();

    let ptr_key = vm.create_string("_ptr");
    vm.table_set_raw(&wrapper, ptr_key, LuaValue::integer(symbol_ptr as i64));

    let name_key = vm.create_string("_name");
    let value = vm.create_string(&name);
    vm.table_set_raw(&wrapper, name_key, value);

    // If we have signature info, store it
    if let Some(sig) = signature {
        let sig_key = vm.create_string("_sig");
        // Store signature as a table for now
        let sig_table = vm.create_table();

        let ret_key = vm.create_string("return");
        vm.table_set_raw(
            &sig_table,
            ret_key,
            LuaValue::integer(sig.return_type.kind as i64),
        );

        let params_key = vm.create_string("params");
        let params_table = vm.create_table();
        for (i, param) in sig.param_types.iter().enumerate() {
            vm.table_set_raw(
                &params_table,
                LuaValue::integer(i as i64 + 1),
                LuaValue::integer(param.kind as i64),
            );
        }
        vm.table_set_raw(&sig_table, params_key, params_table);

        vm.table_set_raw(&wrapper, sig_key, sig_table);
    }

    // Set metatable with __call
    let metatable = vm.create_table();
    let call_key = vm.create_string("__call");
    vm.table_set_raw(&metatable, call_key, LuaValue::cfunction(ffi_call_wrapper));

    vm.table_set_metatable(&wrapper, Some(metatable));

    Ok(MultiValue::single(wrapper))
}

/// Wrapper function for calling C functions via __call metamethod
pub fn ffi_call_wrapper(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Args: self (the function wrapper table), arg1, arg2, ...
    let args = crate::lib_registry::get_args(vm);
    if args.is_empty() {
        return Err(LuaError::RuntimeError(
            "ffi call wrapper: missing self argument".to_string(),
        ));
    }

    let wrapper = &args[0];
    let func_args = &args[1..];

    // Extract function pointer and signature
    unsafe {
        let ptr_key = vm.create_string("_ptr");
        let wrapper_ref_cell = vm
            .get_table(&wrapper)
            .ok_or(LuaError::RuntimeError("Invalid wrapper table".to_string()))?;
        let ptr_val = wrapper_ref_cell.borrow().raw_get(&ptr_key);
        let ptr = if let Some(ptr_value) = ptr_val {
            ptr_value.as_integer().ok_or(LuaError::RuntimeError(
                "ffi call: invalid function pointer".to_string(),
            ))?
        } else {
            return Err(LuaError::RuntimeError(
                "ffi call: function pointer not found".to_string(),
            ));
        } as *mut u8;

        let name_key = vm.create_string("_name");
        let name = if let Some(name_value) = vm.table_get(wrapper, &name_key) {
            let name_str_ptr = name_value.as_string_ptr().ok_or(LuaError::RuntimeError(
                "ffi call: invalid function name".to_string(),
            ))?;
            (*name_str_ptr).as_str().to_string()
        } else {
            return Err(LuaError::RuntimeError(
                "ffi call: function name not found".to_string(),
            ));
        };

        // Try to get signature
        let ffi_state = vm.get_ffi_state();
        let signature = ffi_state.get_function_signature(&name).ok_or_else(|| {
            LuaError::RuntimeError(format!("ffi call: no signature for function '{}'", name))
        })?;

        // Create CFunctionCall and invoke
        let call = CFunctionCall::new(
            ptr,
            signature.return_type.clone(),
            signature.param_types.clone(),
        );

        let result = call.call(func_args)?;
        Ok(MultiValue::single(result))
    }
}

/// ffi.load(name) - Load a shared library
pub fn ffi_load(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let name = crate::lib_registry::get_string(vm, 0, "ffi.load")?;

    let ffi_state = vm.get_ffi_state_mut();
    ffi_state.load_library(&name)?;

    // Return a table that can access library symbols
    let table = vm.create_table();
    // TODO: Add __index metamethod to access symbols

    Ok(MultiValue::single(table))
}

/// ffi.new(ct [, nelem] [, init...]) - Create a cdata object
pub fn ffi_new(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let type_name = crate::lib_registry::get_string(vm, 0, "ffi.new")?;

    // Get or parse the type
    let ffi_state = vm.get_ffi_state();
    let ctype = if let Some(t) = ffi_state.get_type(&type_name) {
        t.clone()
    } else {
        use crate::ffi::parser::parse_base_type;
        parse_base_type(&type_name)?
    };

    // Get optional initialization value
    let init_value = crate::lib_registry::get_arg(vm, 1).unwrap_or(LuaValue::nil());

    // Create CData
    let cdata = if init_value.is_nil() {
        CData::new(ctype.clone())
    } else {
        CData::from_lua_value(ctype.clone(), init_value).map_err(|e| LuaError::RuntimeError(e))?
    };

    // For struct types, create a table with __index and __newindex metamethods
    let table = vm.create_table();

    if matches!(ctype.kind, CTypeKind::Struct) {
        // Store the struct data
        let data_key = vm.create_string("__cdata");
        vm.table_set_raw(&table, data_key, cdata.to_lua_value());

        // Store the type info
        let type_key = vm.create_string("__ctype");
        let type_name_rc = vm.create_string(&type_name);
        vm.table_set_raw(&table, type_key, type_name_rc);

        // Set metamethods for field access
        let metatable = vm.create_table();

        let index_key = vm.create_string("__index");
        vm.table_set_raw(&metatable, index_key, LuaValue::cfunction(ffi_struct_index));

        let newindex_key = vm.create_string("__newindex");
        vm.table_set_raw(
            &metatable,
            newindex_key,
            LuaValue::cfunction(ffi_struct_newindex),
        );

        vm.table_set_metatable(&table, Some(metatable));
    } else {
        // For primitive types, just store the value
        let value_key = vm.create_string("__value");
        vm.table_set_raw(&table, value_key, cdata.to_lua_value());
    }

    Ok(MultiValue::single(table))
}

/// ffi.typeof(ct) - Create a ctype object
pub fn ffi_typeof(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let type_name = crate::lib_registry::get_string(vm, 0, "ffi.typeof")?;

    // Get or parse the type
    let ffi_state = vm.get_ffi_state();
    let ctype = if let Some(t) = ffi_state.get_type(&type_name) {
        t.clone()
    } else {
        use crate::ffi::parser::parse_base_type;
        parse_base_type(&type_name)?
    };

    // Return a table representing the ctype
    let table = vm.create_table();
    let size_key = vm.create_string("size");
    let align_key = vm.create_string("alignment");

    vm.table_set_raw(&table, size_key, LuaValue::integer(ctype.size as i64));
    vm.table_set_raw(&table, align_key, LuaValue::integer(ctype.alignment as i64));

    Ok(MultiValue::single(table))
}

/// ffi.cast(ct, init) - Create a scalar cdata object
pub fn ffi_cast(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let type_name = crate::lib_registry::get_string(vm, 0, "ffi.cast")?;
    let value = crate::lib_registry::get_arg(vm, 1)
        .ok_or_else(|| LuaError::RuntimeError("ffi.cast() requires argument 2".to_string()))?;

    // Get or parse the target type
    let ffi_state = vm.get_ffi_state();
    let ctype = if let Some(t) = ffi_state.get_type(&type_name) {
        t.clone()
    } else {
        use crate::ffi::parser::parse_base_type;
        parse_base_type(&type_name)?
    };

    // Convert value to target type
    let cdata = CData::from_lua_value(ctype, value).map_err(|e| LuaError::RuntimeError(e))?;

    // Return as table (simplified)
    let table = vm.create_table();
    let value_key = vm.create_string("_value");
    vm.table_set_raw(&table, value_key, cdata.to_lua_value());

    Ok(MultiValue::single(table))
}

/// ffi.sizeof(ct [, nelem]) - Get size of ctype
pub fn ffi_sizeof(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let type_name = crate::lib_registry::get_string(vm, 0, "ffi.sizeof")?;

    let ffi_state = vm.get_ffi_state();

    // Try to get predefined type first
    let ctype = if let Some(t) = ffi_state.get_type(&type_name) {
        t.clone()
    } else {
        // Try to parse as a type expression (e.g., "void*", "int[]")
        use crate::ffi::parser::parse_base_type;
        parse_base_type(&type_name)?
    };

    Ok(MultiValue::single(LuaValue::integer(ctype.size as i64)))
}

/// ffi.alignof(ct) - Get alignment of ctype
pub fn ffi_alignof(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let type_name = crate::lib_registry::get_string(vm, 0, "ffi.alignof")?;

    let ffi_state = vm.get_ffi_state();

    // Try to get predefined type first
    let ctype = if let Some(t) = ffi_state.get_type(&type_name) {
        t.clone()
    } else {
        // Try to parse as a type expression
        use crate::ffi::parser::parse_base_type;
        parse_base_type(&type_name)?
    };

    Ok(MultiValue::single(LuaValue::integer(
        ctype.alignment as i64,
    )))
}

/// ffi.offsetof(ct, field) - Get offset of field in struct
pub fn ffi_offsetof(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let type_name = crate::lib_registry::get_string(vm, 0, "ffi.offsetof")?;
    let field_name = crate::lib_registry::get_string(vm, 1, "ffi.offsetof")?;

    let ffi_state = vm.get_ffi_state();
    let ctype = ffi_state
        .get_type(&type_name)
        .ok_or_else(|| LuaError::RuntimeError(format!("Unknown C type: {}", type_name)))?;

    // Get field offset (requires struct type)
    let offset = ctype
        .get_field_offset(&field_name)
        .map_err(|e| LuaError::RuntimeError(e))?;
    Ok(MultiValue::single(LuaValue::integer(offset as i64)))
}

/// ffi.istype(ct, obj) - Test if object is of ctype
pub fn ffi_istype(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let _type_name = crate::lib_registry::get_string(vm, 0, "ffi.istype")?;
    let _obj = crate::lib_registry::get_arg(vm, 1)
        .ok_or_else(|| LuaError::RuntimeError("ffi.istype() requires argument 2".to_string()))?;

    // TODO: Implement proper type checking
    // For now, always return false
    Ok(MultiValue::single(LuaValue::boolean(false)))
}

/// ffi.string(ptr [, len]) - Convert C string to Lua string
pub fn ffi_string(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let ptr_val = crate::lib_registry::get_arg(vm, 0)
        .ok_or_else(|| LuaError::RuntimeError("ffi.string() requires argument 1".to_string()))?;

    let len_val = crate::lib_registry::get_arg(vm, 1);

    // For simplified implementation, convert integer pointer to string
    if let Some(ptr) = ptr_val.as_integer() {
        if ptr == 0 {
            return Err(LuaError::RuntimeError(
                "NULL pointer in ffi.string()".to_string(),
            ));
        }

        unsafe {
            let c_str_ptr = ptr as *const i8;
            let len = if let Some(len_v) = len_val {
                len_v.as_integer().unwrap_or(0) as usize
            } else {
                // Find null terminator
                let mut i = 0;
                while *c_str_ptr.add(i) != 0 {
                    i += 1;
                }
                i
            };

            let slice = std::slice::from_raw_parts(c_str_ptr as *const u8, len);
            let string = String::from_utf8_lossy(slice);
            let lua_str = vm.create_string(&string);
            Ok(MultiValue::single(lua_str))
        }
    } else {
        Err(LuaError::RuntimeError(
            "ffi.string() requires pointer argument".to_string(),
        ))
    }
}

/// ffi.copy(dst, src, len) - Copy data
pub fn ffi_copy(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let dst_val = crate::lib_registry::get_arg(vm, 0)
        .ok_or_else(|| LuaError::RuntimeError("ffi.copy() requires argument 1".to_string()))?;
    let src_val = crate::lib_registry::get_arg(vm, 1)
        .ok_or_else(|| LuaError::RuntimeError("ffi.copy() requires argument 2".to_string()))?;
    let len_val = crate::lib_registry::get_arg(vm, 2)
        .ok_or_else(|| LuaError::RuntimeError("ffi.copy() requires argument 3".to_string()))?;
    let len = len_val
        .as_integer()
        .ok_or_else(|| LuaError::RuntimeError("ffi.copy() length must be integer".to_string()))?
        as usize;

    // Handle pointer to pointer copy
    if let (Some(dst_ptr), Some(src_ptr)) = (dst_val.as_integer(), src_val.as_integer()) {
        if dst_ptr == 0 || src_ptr == 0 {
            return Err(LuaError::RuntimeError(
                "NULL pointer in ffi.copy()".to_string(),
            ));
        }

        unsafe {
            std::ptr::copy_nonoverlapping(src_ptr as *const u8, dst_ptr as *mut u8, len);
        }

        Ok(MultiValue::empty())
    } else {
        // Handle string to pointer copy
        unsafe {
            if let Some(src_str) = src_val.as_string() {
                if let Some(dst_ptr) = dst_val.as_integer() {
                    if dst_ptr == 0 {
                        return Err(LuaError::RuntimeError(
                            "NULL pointer in ffi.copy()".to_string(),
                        ));
                    }

                    let src_bytes = (*src_str).as_str().as_bytes();
                    let copy_len = len.min(src_bytes.len());
                    std::ptr::copy_nonoverlapping(src_bytes.as_ptr(), dst_ptr as *mut u8, copy_len);

                    Ok(MultiValue::empty())
                } else {
                    Err(LuaError::RuntimeError(
                        "ffi.copy() invalid destination".to_string(),
                    ))
                }
            } else {
                Err(LuaError::RuntimeError(
                    "ffi.copy() invalid arguments".to_string(),
                ))
            }
        }
    }
}

/// ffi.fill(dst, len [, c]) - Fill memory
pub fn ffi_fill(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let dst_val = crate::lib_registry::get_arg(vm, 0)
        .ok_or_else(|| LuaError::RuntimeError("ffi.fill() requires argument 1".to_string()))?;
    let len_val = crate::lib_registry::get_arg(vm, 1)
        .ok_or_else(|| LuaError::RuntimeError("ffi.fill() requires argument 2".to_string()))?;

    let len = len_val
        .as_integer()
        .ok_or_else(|| LuaError::RuntimeError("ffi.fill() length must be integer".to_string()))?
        as usize;
    let fill_byte = if let Some(c_val) = crate::lib_registry::get_arg(vm, 2) {
        c_val.as_integer().unwrap_or(0) as u8
    } else {
        0u8
    };

    if let Some(dst_ptr) = dst_val.as_integer() {
        if dst_ptr == 0 {
            return Err(LuaError::RuntimeError(
                "NULL pointer in ffi.fill()".to_string(),
            ));
        }

        unsafe {
            std::ptr::write_bytes(dst_ptr as *mut u8, fill_byte, len);
        }

        Ok(MultiValue::empty())
    } else {
        Err(LuaError::RuntimeError(
            "ffi.fill() requires pointer argument".to_string(),
        ))
    }
}

/// __index metamethod for struct field access
pub fn ffi_struct_index(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = crate::lib_registry::get_args(vm);
    if args.len() < 2 {
        return Err(LuaError::RuntimeError(
            "struct __index requires 2 arguments".to_string(),
        ));
    }

    let struct_table = &args[0];
    let field_name_val = &args[1];

    let field_name = unsafe {
        if let Some(s) = field_name_val.as_string_ptr() {
            (*s).as_str()
        } else {
            return Err(LuaError::RuntimeError(
                "struct field name must be string".to_string(),
            ));
        }
    };

    // Create all string keys upfront
    let type_key = vm.create_string("__ctype");
    let fields_key = vm.create_string("__fields");
    let field_key = vm.create_string(field_name);

    // Get the struct type
    unsafe {
        let table_ref_cell = vm
            .get_table(&struct_table)
            .ok_or(LuaError::RuntimeError("Invalid struct table".to_string()))?;
        let type_name_val = table_ref_cell.borrow().raw_get(&type_key);

        let type_name = if let Some(tn) = type_name_val {
            if let Some(s) = tn.as_string_ptr() {
                (*s).as_str()
            } else {
                return Err(LuaError::RuntimeError("invalid struct type".to_string()));
            }
        } else {
            return Err(LuaError::RuntimeError("struct type not found".to_string()));
        };

        // Get the ctype
        let ffi_state = vm.get_ffi_state();
        let ctype = ffi_state
            .get_type(&type_name)
            .ok_or_else(|| LuaError::RuntimeError(format!("Unknown type: {}", type_name)))?;

        // Get field info
        let fields = ctype
            .fields
            .as_ref()
            .ok_or(LuaError::RuntimeError("not a struct type".to_string()))?;
        let field = fields
            .get(field_name)
            .ok_or_else(|| LuaError::RuntimeError(format!("Field '{}' not found", field_name)))?;

        // Get stored field values
        let fields_table_val = table_ref_cell.borrow().raw_get(&fields_key);

        if let Some(ft) = fields_table_val {
            if ft.is_table() {
                let fields_ref_cell = vm
                    .get_table(&ft)
                    .ok_or(LuaError::RuntimeError("Invalid fields table".to_string()))?;
                let field_val = fields_ref_cell.borrow().raw_get(&field_key);
                if let Some(fv) = field_val {
                    return Ok(MultiValue::single(fv));
                }
            }
        }

        // Return default value based on type
        match field.ctype.kind {
            CTypeKind::Int8
            | CTypeKind::Int16
            | CTypeKind::Int32
            | CTypeKind::Int64
            | CTypeKind::UInt8
            | CTypeKind::UInt16
            | CTypeKind::UInt32
            | CTypeKind::UInt64 => Ok(MultiValue::single(LuaValue::integer(0))),
            CTypeKind::Float | CTypeKind::Double => Ok(MultiValue::single(LuaValue::number(0.0))),
            CTypeKind::Pointer => Ok(MultiValue::single(LuaValue::integer(0))),
            _ => Ok(MultiValue::single(LuaValue::nil())),
        }
    }
}

/// __newindex metamethod for struct field assignment
pub fn ffi_struct_newindex(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = crate::lib_registry::get_args(vm);
    if args.len() < 3 {
        return Err(LuaError::RuntimeError(
            "struct __newindex requires 3 arguments".to_string(),
        ));
    }

    let struct_table = &args[0];
    let field_name_val = &args[1];
    let value = &args[2];

    let field_name = unsafe {
        if let Some(s) = field_name_val.as_string_ptr() {
            (*s).as_str()
        } else {
            return Err(LuaError::RuntimeError(
                "struct field name must be string".to_string(),
            ));
        }
    };

    // Get the struct type and validate field exists
    unsafe {
        let type_key = vm.create_string("__ctype");
        let type_name_val = vm.table_get(struct_table, &type_key);

        let type_name = if let Some(tn) = type_name_val {
            if let Some(s) = tn.as_string_ptr() {
                (*s).as_str().to_string()
            } else {
                return Err(LuaError::RuntimeError("invalid struct type".to_string()));
            }
        } else {
            return Err(LuaError::RuntimeError("struct type not found".to_string()));
        };

        // Validate field exists
        let ffi_state = vm.get_ffi_state();
        let ctype = ffi_state
            .get_type(&type_name)
            .ok_or_else(|| LuaError::RuntimeError(format!("Unknown type: {}", type_name)))?;

        let fields = ctype
            .fields
            .as_ref()
            .ok_or(LuaError::RuntimeError("not a struct type".to_string()))?;

        if !fields.contains_key(field_name) {
            return Err(LuaError::RuntimeError(format!(
                "Field '{}' not found",
                field_name
            )));
        }

        // Get or create __fields table
        let fields_key = vm.create_string("__fields");
        let fields_table_val = vm.table_get(struct_table, &fields_key);

        let fields_table_value = if let Some(ft) = fields_table_val {
            if ft.as_table_id().is_some() {
                ft
            } else {
                return Err(LuaError::RuntimeError("invalid fields table".to_string()));
            }
        } else {
            // Create fields table
            let new_fields = vm.create_table();
            vm.table_set_raw(struct_table, fields_key, new_fields.clone());
            new_fields
        };

        // Set the field value
        let field_key = vm.create_string(field_name);
        let fields_ref = vm
            .get_table(&fields_table_value)
            .ok_or(LuaError::RuntimeError("Invalid fields table".to_string()))?;
        fields_ref.borrow_mut().raw_set(field_key, value.clone());

        Ok(MultiValue::empty())
    }
}
