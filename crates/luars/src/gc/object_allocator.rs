// Object Pool V3 - Simplified high-performance design
//
// Key Design Principles:
// 1. All IDs are u32 indices into Vec storage
// 2. Small objects (String, Function, Upvalue) use Vec<Option<T>>
// 3. Large objects (Table, Thread) use Vec<Option<Box<T>>> to avoid copy on resize
// 4. No chunking overhead - direct Vec indexing for O(1) access
// 5. Free list for slot reuse
// 6. GC headers embedded in objects for mark-sweep

use crate::gc::string_interner::StringInterner;
use crate::lua_value::{CClosureFunction, Chunk, LuaUpvalue, LuaUserdata};
use crate::lua_vm::{CFunction, LuaState};
use crate::{
    GC, GcBinary, GcCClosure, GcFunction, GcObjectOwner, GcTable, GcThread, GcUpvalue, GcUserdata,
    Instruction, LuaFunction, LuaResult, LuaTable, LuaValue, StringPtr, UpvaluePtr,
};
use std::rc::Rc;

pub type CreateResult = LuaResult<LuaValue>;

/// High-performance object pool for the Lua VM
/// - Small objects (String, Function, Upvalue) use Pool<T> with direct Vec storage
/// - Large objects (Table, Thread) use BoxPool<T> to avoid copy on resize
/// - ALL strings are interned via StringInterner for O(1) equality checks
pub struct ObjectAllocator {
    strings: StringInterner, // Private - use create_string() to intern
}

impl ObjectAllocator {
    pub fn new() -> Self {
        let pool = Self {
            strings: StringInterner::new(),
        };

        pool
    }

    // ==================== String Operations ====================

    /// Create or intern a string (Lua-style with proper hash collision handling)
    ///
    #[inline]
    pub fn create_string(&mut self, gc: &mut GC, s: &str) -> CreateResult {
        self.strings.intern(s, gc)
    }

    /// Create string from owned String (avoids clone if already interned)
    ///
    #[inline]
    pub fn create_string_owned(&mut self, gc: &mut GC, s: String) -> CreateResult {
        self.strings.intern(&s, gc)
    }

    /// Create a binary value from Vec<u8>
    ///
    #[inline]
    pub fn create_binary(&mut self, gc: &mut GC, data: Vec<u8>) -> CreateResult {
        let current_white = gc.current_white;
        let size = (std::mem::size_of::<GcBinary>() + data.len()) as u32;
        let gc_ptr = Box::new(GcBinary::new(data, current_white, size));
        let gc_binary = GcObjectOwner::Binary(gc_ptr);
        let ptr = gc_binary.as_binary_ptr().unwrap();
        gc.trace_object(gc_binary)?;
        Ok(LuaValue::binary(ptr))
    }

    /// Create a substring from an existing string (optimized for string.sub)
    /// Returns the original string ID if the range covers the entire string.
    /// With complete interning, substrings are automatically deduplicated.
    ///
    #[inline]
    pub fn create_substring(
        &mut self,
        gc: &mut GC,
        s_value: LuaValue,
        start: usize,
        end: usize,
    ) -> CreateResult {
        // Get bytes - handle both string and binary types
        let bytes = if let Some(s) = s_value.as_str() {
            s.as_bytes()
        } else if let Some(b) = s_value.as_binary() {
            b
        } else {
            return self.create_string(gc, "");
        };

        // Extract substring info
        // Clamp indices
        let start = start.min(bytes.len());
        let end = end.min(bytes.len());

        if start >= end {
            return self.create_string(gc, "");
        }

        // Fast path: return original if full range
        if start == 0 && end == bytes.len() {
            return Ok(s_value);
        }

        // Extract the byte range
        let substring_bytes = &bytes[start..end];

        // Try to create a valid UTF-8 string from these bytes
        // If invalid, create a binary value to preserve the original bytes
        match std::str::from_utf8(substring_bytes) {
            Ok(valid_str) => {
                // Valid UTF-8 - intern as string
                self.create_string(gc, valid_str)
            }
            Err(_) => {
                // Invalid UTF-8 - create binary value to preserve original bytes
                // This is important for binary data like bytecode
                self.create_binary(gc, substring_bytes.to_vec())
            }
        }
    }

    // ==================== Table Operations ====================

    #[inline]
    pub fn create_table(
        &mut self,
        gc: &mut GC,
        array_size: usize,
        hash_size: usize,
    ) -> CreateResult {
        // Lua 5.5 ltable.c luaH_size:
        //   lu_mem sz = sizeof(Table) + concretesize(t->asize);
        //   if (!isdummy(t)) sz += sizehash(t);
        //
        // concretesize(size) = size * (sizeof(Value) + 1) + sizeof(unsigned)
        //   = size * (16 + 1) + 4 = size * 17 + 4
        //
        // sizehash(t) = sizenode(t) * sizeof(Node) + extraLastfree(t)
        //   ≈ (1 << lsizenode) * 24 + (has_lastfree ? 8 : 0)
        //   For simplicity, use hash_size * 24
        //
        // sizeof(Table) ≈ 80 bytes (base struct)
        let current_white = gc.current_white;
        let base_size = std::mem::size_of::<GcTable>();
        let array_bytes = if array_size > 0 {
            array_size * 17 + 4
        } else {
            0
        };
        let hash_bytes = if hash_size > 0 {
            hash_size * 24 + 8 // Node size + lastfree overhead
        } else {
            0
        };
        let size = (base_size + array_bytes + hash_bytes) as u32;
        let ptr = Box::new(GcTable::new(
            LuaTable::new(array_size as u32, hash_size as u32),
            current_white,
            size,
        ));
        let gc_table = GcObjectOwner::Table(ptr);
        let ptr = gc_table.as_table_ptr().unwrap();
        gc.trace_object(gc_table)?;
        Ok(LuaValue::table(ptr))
    }

    // ==================== Function Operations ====================

    /// Create a Lua function (closure with bytecode chunk)
    /// Now caches upvalue pointers for direct access
    ///
    #[inline]
    pub fn create_function(
        &mut self,
        gc: &mut GC,
        chunk: Rc<Chunk>,
        upvalue_ptrs: Vec<UpvaluePtr>,
    ) -> CreateResult {
        let current_white = gc.current_white;
        // Calculate size: base + upvalues + chunk data
        // TODO: refine size calculation
        let upval_size = upvalue_ptrs.len() * std::mem::size_of::<UpvaluePtr>();
        let instr_size = chunk.code.len() * std::mem::size_of::<Instruction>();
        let const_size = chunk.constants.len() * std::mem::size_of::<LuaValue>();
        let child_size = chunk.child_protos.len() * std::mem::size_of::<Chunk>();
        let line_size = chunk.line_info.len() * std::mem::size_of::<u32>();
        let size = (upval_size + instr_size + const_size + child_size + line_size) as u32;

        let gc_func = GcObjectOwner::Function(Box::new(GcFunction::new(
            LuaFunction::new(chunk, upvalue_ptrs),
            current_white,
            size,
        )));
        let ptr = gc_func.as_function_ptr().unwrap();
        gc.trace_object(gc_func)?;
        Ok(LuaValue::function(ptr))
    }

    /// Create a C closure (native function with upvalues)
    /// Now caches upvalue pointers for direct access
    ///
    #[inline]
    pub fn create_c_closure(
        &mut self,
        gc: &mut GC,
        func: CFunction,
        upvalues: Vec<LuaValue>,
    ) -> CreateResult {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<CFunction>() as u32
            + (upvalues.len() as u32 * std::mem::size_of::<LuaValue>() as u32);
        let gc_func = GcObjectOwner::CClosure(Box::new(GcCClosure::new(
            CClosureFunction::new(func, upvalues),
            current_white,
            size,
        )));
        let ptr = gc_func.as_closure_ptr().unwrap();
        gc.trace_object(gc_func)?;
        Ok(LuaValue::cclosure(ptr))
    }

    // ==================== Upvalue Operations ====================

    /// Create upvalue from LuaUpvalue
    ///
    pub fn create_upvalue(&mut self, gc: &mut GC, upvalue: LuaUpvalue) -> LuaResult<UpvaluePtr> {
        let current_white = gc.current_white;
        let size = 64;
        let mut boxed = Box::new(GcUpvalue::new(upvalue, current_white, size));
        // Fix up closed upvalue's v pointer to its own closed_value field.
        // Must happen after boxing so the heap address is stable.
        // No-op for open upvalues (v already points to valid stack slot).
        boxed.data.fix_closed_ptr();
        let gc_uv = GcObjectOwner::Upvalue(boxed);
        let ptr = gc_uv.as_upvalue_ptr().unwrap();
        gc.trace_object(gc_uv)?;
        Ok(ptr)
    }

    // ==================== Userdata Operations ====================

    #[inline]
    pub fn create_userdata(&mut self, gc: &mut GC, userdata: LuaUserdata) -> CreateResult {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<LuaUserdata>();
        let gc_userdata = GcObjectOwner::Userdata(Box::new(GcUserdata::new(
            userdata,
            current_white,
            size as u32,
        )));
        let ptr = gc_userdata.as_userdata_ptr().unwrap();
        gc.trace_object(gc_userdata)?;
        Ok(LuaValue::userdata(ptr))
    }

    // ==================== Thread Operations ====================

    #[inline]
    pub fn create_thread(&mut self, gc: &mut GC, thread: LuaState) -> CreateResult {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<LuaState>();
        let mut gc_thread =
            GcObjectOwner::Thread(Box::new(GcThread::new(thread, current_white, size as u32)));
        let ptr = gc_thread.as_thread_ptr().unwrap();
        unsafe {
            gc_thread.as_thread_mut().unwrap().set_thread_ptr(ptr);
        }

        gc.trace_object(gc_thread)?;
        Ok(LuaValue::thread(ptr))
    }

    #[inline]
    pub fn remove_str(&mut self, str_ptr: StringPtr) {
        self.strings.remove_dead_intern(str_ptr);
    }
}
