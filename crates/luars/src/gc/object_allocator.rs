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
use crate::lua_value::UpvalueStore;
use crate::lua_value::{
    CClosureFunction, LuaProto, LuaUpvalue, LuaUserdata, RClosureFunction, RustCallback,
};
use crate::lua_vm::{CFunction, LuaState};
use crate::{
    LuaFunction, LuaResult, LuaTable, LuaValue,
    gc::{
        GC, GcCClosure, GcFunction, GcObjectOwner, GcProto, GcRClosure, GcTable, GcThread,
        GcUpvalue, GcUserdata, ProtoPtr, StringPtr, UpvaluePtr,
    },
};

pub type CreateResult = LuaResult<LuaValue>;

/// High-performance object pool for the Lua VM
/// - Small objects (String, Function, Upvalue) use Pool<T> with direct Vec storage
/// - Large objects (Table, Thread) use BoxPool<T> to avoid copy on resize
/// - ALL strings are interned via StringInterner for O(1) equality checks
pub struct ObjectAllocator {
    strings: StringInterner, // Private - use create_string() to intern
}

impl Default for ObjectAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectAllocator {
    pub fn new() -> Self {
        Self {
            strings: StringInterner::new(),
        }
    }

    // ==================== String Operations ====================

    /// Create or intern a string (Lua-style with proper hash collision handling)
    ///
    #[inline]
    pub fn create_string(&mut self, gc: &mut GC, s: &str) -> CreateResult {
        self.strings.intern(s, gc)
    }

    /// Create string from owned String (avoids clone if not already interned)
    ///
    #[inline]
    pub fn create_string_owned(&mut self, gc: &mut GC, s: String) -> CreateResult {
        self.strings.intern_owned(s, gc)
    }

    /// Create a Lua string-like value from raw bytes.
    /// All short byte strings are interned so Lua string equality keeps its fast path.
    #[inline]
    pub fn create_bytes(&mut self, gc: &mut GC, bytes: &[u8]) -> CreateResult {
        self.strings.intern_bytes(bytes, gc)
    }

    /// Create a raw byte string from Vec<u8> without requiring UTF-8.
    /// This compatibility path now uses the same byte-string interning rules as `create_bytes`.
    #[inline]
    pub fn create_binary(&mut self, gc: &mut GC, data: Vec<u8>) -> CreateResult {
        self.strings.intern_bytes_owned(data, gc)
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
        let source_is_ascii = s_value.as_str().is_some_and(str::is_ascii);
        let Some(bytes) = s_value.as_bytes() else {
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

        // If source is already a valid UTF-8 string, use unchecked conversion
        // since a substring of valid UTF-8 at valid char boundaries is also valid.
        // For arbitrary byte indices (Lua semantics), validate.
        if source_is_ascii {
            // SAFETY: a substring of ASCII bytes is always valid ASCII/UTF-8.
            self.create_string(gc, unsafe {
                std::str::from_utf8_unchecked(substring_bytes)
            })
        } else {
            self.create_bytes(gc, substring_bytes)
        }
    }

    // ==================== Table Operations ====================

    #[inline(always)]
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

    #[inline(always)]
    pub fn create_proto(&mut self, gc: &mut GC, chunk: LuaProto) -> LuaResult<ProtoPtr> {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<GcProto>() as u32 + chunk.proto_data_size;
        let gc_proto = GcObjectOwner::Proto(Box::new(GcProto::new(chunk, current_white, size)));
        let ptr = gc_proto.as_proto_ptr().unwrap();
        gc.trace_object(gc_proto)?;
        Ok(ptr)
    }

    /// Create a Lua function (closure with bytecode chunk)
    /// Now caches upvalue pointers for direct access
    ///
    #[inline(always)]
    pub fn create_function(
        &mut self,
        gc: &mut GC,
        chunk: ProtoPtr,
        upvalue_store: UpvalueStore,
    ) -> CreateResult {
        let current_white = gc.current_white;
        let upval_size = upvalue_store.len() * std::mem::size_of::<UpvaluePtr>();
        let size = std::mem::size_of::<GcFunction>() as u32 + upval_size as u32;

        let gc_func = GcObjectOwner::Function(Box::new(GcFunction::new(
            LuaFunction::new(chunk, upvalue_store),
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

    /// Create an RClosure (Rust closure with captured state + optional upvalues)
    /// Unlike C closures which store a bare fn pointer, this stores a Box<dyn Fn>
    /// that can capture arbitrary Rust state.
    #[inline]
    pub fn create_rclosure(
        &mut self,
        gc: &mut GC,
        func: RustCallback,
        upvalues: Vec<LuaValue>,
    ) -> CreateResult {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<GcRClosure>() as u32
            + (upvalues.len() as u32 * std::mem::size_of::<LuaValue>() as u32);
        let gc_func = GcObjectOwner::RClosure(Box::new(GcRClosure::new(
            RClosureFunction::new(func, upvalues),
            current_white,
            size,
        )));
        let ptr = gc_func.as_rclosure_ptr().unwrap();
        gc.trace_object(gc_func)?;
        Ok(LuaValue::rclosure(ptr))
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
