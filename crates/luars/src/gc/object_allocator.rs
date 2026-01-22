// Object Pool V3 - Simplified high-performance design
//
// Key Design Principles:
// 1. All IDs are u32 indices into Vec storage
// 2. Small objects (String, Function, Upvalue) use Vec<Option<T>>
// 3. Large objects (Table, Thread) use Vec<Option<Box<T>>> to avoid copy on resize
// 4. No chunking overhead - direct Vec indexing for O(1) access
// 5. Free list for slot reuse
// 6. GC headers embedded in objects for mark-sweep

use crate::gc::gc_object::FunctionBody;
use crate::gc::string_interner::StringInterner;
use crate::lua_value::{Chunk, LuaUpvalue, LuaUserdata};
use crate::lua_vm::{CFunction, LuaState, SafeOption};
use crate::{
    GC, GcBinary, GcFunction, GcObjectOwner, GcTable, GcThread, GcUpvalue, GcUserdata, LuaTable,
    LuaValue, StringPtr, Upvalue, UpvaluePtr,
};
use std::rc::Rc;

/// High-performance object pool for the Lua VM
/// - Small objects (String, Function, Upvalue) use Pool<T> with direct Vec storage
/// - Large objects (Table, Thread) use BoxPool<T> to avoid copy on resize
/// - ALL strings are interned via StringInterner for O(1) equality checks
pub struct ObjectAllocator {
    strings: StringInterner,   // Private - use create_string() to intern
    short_string_limit: usize, // Obsolete - kept for backwards compatibility
}

impl ObjectAllocator {
    pub fn new(option: SafeOption) -> Self {
        let pool = Self {
            strings: StringInterner::new(option.short_string_limit),
            short_string_limit: option.short_string_limit,
        };

        pool
    }

    /// Get short string limit (now obsolete - all strings are interned)
    /// Kept for backwards compatibility, returns a default value
    pub fn get_short_string_limit(&self) -> usize {
        self.short_string_limit
    }
    // ==================== String Operations ====================

    /// Create or intern a string (Lua-style with proper hash collision handling)
    ///
    #[inline]
    pub fn create_string(&mut self, gc: &mut GC, s: &str) -> LuaValue {
        self.strings.intern(s, gc)
    }

    /// Create string from owned String (avoids clone if already interned)
    ///
    #[inline]
    pub fn create_string_owned(&mut self, gc: &mut GC, s: String) -> LuaValue {
        self.strings.intern(&s, gc)
    }

    /// Create a binary value from Vec<u8>
    ///
    #[inline]
    pub fn create_binary(&mut self, gc: &mut GC, data: Vec<u8>) -> LuaValue {
        let current_white = gc.current_white;
        let size = (std::mem::size_of::<GcBinary>() + data.len()) as u32;
        let gc_ptr = Box::new(GcBinary::new(data, current_white, size));
        let gc_binary = GcObjectOwner::Binary(gc_ptr);
        let ptr = gc_binary.as_binary_ptr().unwrap();
        gc.trace_object(gc_binary);
        LuaValue::binary(ptr)
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
    ) -> LuaValue {
        let string = match s_value.as_str() {
            Some(s) => s,
            None => return self.create_string(gc, ""),
        };
        // Extract substring info first
        let substring = {
            // Clamp indices
            let start = start.min(string.len());
            let end = end.min(string.len());

            if start >= end {
                return self.create_string(gc, "");
            }

            // Fast path: return original if full range
            if start == 0 && end == string.len() {
                return s_value;
            }

            // Copy substring to avoid borrowing issue
            &string[start..end]
        };

        // Intern the substring - will be deduplicated if it already exists
        self.create_string(gc, substring)
    }

    // ==================== Table Operations ====================

    #[inline]
    pub fn create_table(&mut self, gc: &mut GC, array_size: usize, hash_size: usize) -> LuaValue {
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
        gc.trace_object(gc_table);
        LuaValue::table(ptr)
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
    ) -> LuaValue {
        let current_white = gc.current_white;
        // Calculate size: base + upvalues + chunk data
        // TODO: refine size calculation
        let upvalue_count = upvalue_ptrs.len();
        let instr_size = chunk.code.len() * 8;
        let const_size = chunk.constants.len() * 32;
        let child_size = chunk.child_protos.len() * std::mem::size_of::<Chunk>();
        let line_size = chunk.line_info.len() * 4;
        let size =
            (256 + upvalue_count * 64 + instr_size + const_size + child_size + line_size + 512)
                as u32;

        let gc_func = GcObjectOwner::Function(Box::new(GcFunction::new(
            FunctionBody::Lua(chunk, upvalue_ptrs),
            current_white,
            size,
        )));
        let ptr = gc_func.as_function_ptr().unwrap();
        gc.trace_object(gc_func);
        LuaValue::function(ptr)
    }

    /// Create a C closure (native function with upvalues)
    /// Now caches upvalue pointers for direct access
    ///
    #[inline]
    pub fn create_c_closure(
        &mut self,
        gc: &mut GC,
        func: CFunction,
        upvalue_ptrs: Vec<UpvaluePtr>,
    ) -> LuaValue {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<CFunction>() as u32 + (upvalue_ptrs.len() as u32 * 64);
        let gc_func = GcObjectOwner::Function(Box::new(GcFunction::new(
            FunctionBody::CClosure(func, upvalue_ptrs),
            current_white,
            size,
        )));
        let ptr = gc_func.as_function_ptr().unwrap();
        gc.trace_object(gc_func);
        LuaValue::function(ptr)
    }

    // ==================== Upvalue Operations ====================

    /// Create an open upvalue pointing to a stack location
    ///
    #[inline]
    pub fn create_upvalue_open(&mut self, gc: &mut GC, stack_index: usize) -> UpvaluePtr {
        let current_white = gc.current_white;
        let upvalue = Upvalue::Open(stack_index);
        let size = 64;
        let gc_uv = GcObjectOwner::Upvalue(Box::new(GcUpvalue::new(upvalue, current_white, size)));
        let ptr = gc_uv.as_upvalue_ptr().unwrap();
        gc.trace_object(gc_uv);
        ptr
    }

    /// Create a closed upvalue with a value
    ///
    #[inline]
    pub fn create_upvalue_closed(&mut self, gc: &mut GC, value: LuaValue) -> UpvaluePtr {
        let current_white = gc.current_white;
        let upvalue = Upvalue::Closed(value);
        let size = 64;
        let gc_uv = GcObjectOwner::Upvalue(Box::new(GcUpvalue::new(upvalue, current_white, size)));
        let ptr = gc_uv.as_upvalue_ptr().unwrap();
        gc.trace_object(gc_uv);
        ptr
    }

    /// Create upvalue from LuaUpvalue
    ///
    pub fn create_upvalue(
        &mut self,
        gc: &mut GC,
        upvalue: Rc<LuaUpvalue>,
    ) -> UpvaluePtr {
        // Check if open and get stack index
        if upvalue.is_open() {
            self.create_upvalue_open(gc, upvalue.get_stack_index().unwrap_or(0))
        } else {
            self.create_upvalue_closed(
                gc,
                upvalue.get_closed_value().unwrap_or(LuaValue::nil()),
            )
        }
    }

    // ==================== Userdata Operations ====================

    #[inline]
    pub fn create_userdata(&mut self, gc: &mut GC, userdata: LuaUserdata) -> LuaValue {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<LuaUserdata>();
        let gc_userdata = GcObjectOwner::Userdata(Box::new(GcUserdata::new(
            userdata,
            current_white,
            size as u32,
        )));
        let ptr = gc_userdata.as_userdata_ptr().unwrap();
        gc.trace_object(gc_userdata);
        LuaValue::userdata(ptr)
    }

    // ==================== Thread Operations ====================

    #[inline]
    pub fn create_thread(&mut self, gc: &mut GC, thread: LuaState) -> LuaValue {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<LuaState>();
        let mut gc_thread =
            GcObjectOwner::Thread(Box::new(GcThread::new(thread, current_white, size as u32)));
        let ptr = gc_thread.as_thread_ptr().unwrap();
        unsafe {
            gc_thread.as_thread_mut().unwrap().set_thread_ptr(ptr);
        }

        gc.trace_object(gc_thread);
        LuaValue::thread(ptr)
    }

    #[inline]
    pub fn remove_str(&mut self, str_ptr: StringPtr) {
        self.strings.remove_dead_intern(str_ptr);
    }
}
