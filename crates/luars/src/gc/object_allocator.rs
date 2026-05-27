use crate::gc::string_interner::StringInterner;
use crate::lua_value::UpvalueStore;
use crate::lua_value::{
    CClosureFunction, LuaProto, LuaUpvalue, LuaUserdata, RClosureFunction, RustCallback,
};
use crate::lua_vm::{CFunction, LuaState};
use crate::{
    LuaRawFunction, LuaRawTable, LuaResult, LuaValue,
    gc::{
        GC, GcCClosure, GcFunction, GcObjectOwner, GcProto, GcRClosure, GcString, GcTable,
        GcThread, GcUpvalue, GcUserdata, PagedPool, ProtoPtr, StringPtr, TableAllocHandle,
        UpvaluePtr,
    },
};

pub type CreateResult = LuaResult<LuaValue>;

#[derive(Debug, Clone, Default)]
pub struct ObjectAllocatorBreakdown {
    pub string_interner_used: usize,
    pub string_interner_dead: usize,
    pub string_interner_capacity: usize,
    pub string_interner_bytes: usize,
    pub string_pool_bytes: usize,
    pub table_pool_bytes: usize,
    pub function_pool_bytes: usize,
    pub cclosure_pool_bytes: usize,
    pub rclosure_pool_bytes: usize,
    pub upvalue_pool_bytes: usize,
    pub userdata_pool_bytes: usize,
    pub proto_pool_bytes: usize,
    pub pool_total_slots: usize,
    pub pool_live_slots: usize,
    pub pool_free_slots: usize,
    pub pool_page_count: usize,
    pub table_cached_hash_bytes: usize,
    pub table_cached_hash_blocks: usize,
    pub table_cached_array_bytes: usize,
    pub table_cached_array_blocks: usize,
}

pub struct ObjectAllocator {
    strings: StringInterner, // Private - use create_string() to intern
    string_pool: PagedPool<GcString>,
    table_pool: PagedPool<GcTable>,
    function_pool: PagedPool<GcFunction>,
    cclosure_pool: PagedPool<GcCClosure>,
    rclosure_pool: PagedPool<GcRClosure>,
    upvalue_pool: PagedPool<GcUpvalue>,
    userdata_pool: PagedPool<GcUserdata>,
    proto_pool: PagedPool<GcProto>,
    table_allocator: TableAllocHandle,
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
            string_pool: PagedPool::new(128),
            table_pool: PagedPool::new(64),
            function_pool: PagedPool::new(32),
            cclosure_pool: PagedPool::new(16),
            rclosure_pool: PagedPool::new(16),
            upvalue_pool: PagedPool::new(64),
            userdata_pool: PagedPool::new(16),
            proto_pool: PagedPool::new(16),
            table_allocator: TableAllocHandle::default(),
        }
    }

    /// Create or intern a string (Lua-style with proper hash collision handling)
    ///
    #[inline]
    pub fn create_string(&mut self, gc: &mut GC, s: &str) -> CreateResult {
        self.strings.intern(s, gc, &mut self.string_pool)
    }

    /// Create string from owned String (avoids clone if not already interned)
    ///
    #[inline]
    pub fn create_string_owned(&mut self, gc: &mut GC, s: String) -> CreateResult {
        self.strings.intern_owned(s, gc, &mut self.string_pool)
    }

    /// Create a Lua string-like value from raw bytes.
    /// All short byte strings are interned so Lua string equality keeps its fast path.
    #[inline]
    pub fn create_bytes(&mut self, gc: &mut GC, bytes: &[u8]) -> CreateResult {
        self.strings.intern_bytes(bytes, gc, &mut self.string_pool)
    }

    /// Create a raw byte string from Vec<u8> without requiring UTF-8.
    /// This compatibility path now uses the same byte-string interning rules as `create_bytes`.
    #[inline]
    pub fn create_binary(&mut self, gc: &mut GC, data: Vec<u8>) -> CreateResult {
        self.strings
            .intern_bytes_owned(data, gc, &mut self.string_pool)
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
        let ptr = self.table_pool.alloc(GcTable::new(
            LuaRawTable::new(
                array_size as u32,
                hash_size as u32,
                self.table_allocator.clone(),
            ),
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
        let gc_proto = GcObjectOwner::Proto(self.proto_pool.alloc(GcProto::new(
            chunk,
            current_white,
            size,
        )));
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

        let gc_func = GcObjectOwner::Function(self.function_pool.alloc(GcFunction::new(
            LuaRawFunction::new(chunk, upvalue_store),
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
        let gc_func = GcObjectOwner::CClosure(self.cclosure_pool.alloc(GcCClosure::new(
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
        let gc_func = GcObjectOwner::RClosure(self.rclosure_pool.alloc(GcRClosure::new(
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
        let mut pooled = self
            .upvalue_pool
            .alloc(GcUpvalue::new(upvalue, current_white, size));
        // Fix up closed upvalue's v pointer to its own closed_value field.
        // Must happen after boxing so the heap address is stable.
        // No-op for open upvalues (v already points to valid stack slot).
        pooled.data.fix_closed_ptr();
        let gc_uv = GcObjectOwner::Upvalue(pooled);
        let ptr = gc_uv.as_upvalue_ptr().unwrap();
        gc.trace_object(gc_uv)?;
        Ok(ptr)
    }

    // ==================== Userdata Operations ====================

    #[inline]
    pub fn create_userdata(&mut self, gc: &mut GC, userdata: LuaUserdata) -> CreateResult {
        let current_white = gc.current_white;
        let size = std::mem::size_of::<LuaUserdata>();
        let gc_userdata = GcObjectOwner::Userdata(self.userdata_pool.alloc(GcUserdata::new(
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
        gc_thread.as_thread_mut().unwrap().set_thread_ptr(ptr);

        gc.trace_object(gc_thread)?;
        Ok(LuaValue::thread(ptr))
    }

    #[inline]
    pub fn remove_str(&mut self, str_ptr: StringPtr) {
        self.strings.remove_dead_intern(str_ptr);
    }

    pub fn trim_after_full_gc(&mut self) {
        self.table_pool.release_empty_pages();
        self.table_allocator.clear_cached_blocks();
    }

    pub fn breakdown(&self) -> ObjectAllocatorBreakdown {
        let string_interner = self.strings.memory_stats();
        let string_pool = self.string_pool.stats();
        let table_pool = self.table_pool.stats();
        let function_pool = self.function_pool.stats();
        let cclosure_pool = self.cclosure_pool.stats();
        let rclosure_pool = self.rclosure_pool.stats();
        let upvalue_pool = self.upvalue_pool.stats();
        let userdata_pool = self.userdata_pool.stats();
        let proto_pool = self.proto_pool.stats();
        let table_alloc = self.table_allocator.stats();

        let pool_live_slots = string_pool.live_slots
            + table_pool.live_slots
            + function_pool.live_slots
            + cclosure_pool.live_slots
            + rclosure_pool.live_slots
            + upvalue_pool.live_slots
            + userdata_pool.live_slots
            + proto_pool.live_slots;
        let pool_free_slots = string_pool.free_slots
            + table_pool.free_slots
            + function_pool.free_slots
            + cclosure_pool.free_slots
            + rclosure_pool.free_slots
            + upvalue_pool.free_slots
            + userdata_pool.free_slots
            + proto_pool.free_slots;
        let pool_page_count = string_pool.page_count
            + table_pool.page_count
            + function_pool.page_count
            + cclosure_pool.page_count
            + rclosure_pool.page_count
            + upvalue_pool.page_count
            + userdata_pool.page_count
            + proto_pool.page_count;

        let pool_total_slots = string_pool.total_slots
            + table_pool.total_slots
            + function_pool.total_slots
            + cclosure_pool.total_slots
            + rclosure_pool.total_slots
            + upvalue_pool.total_slots
            + userdata_pool.total_slots
            + proto_pool.total_slots;

        ObjectAllocatorBreakdown {
            string_interner_used: string_interner.used_entries,
            string_interner_dead: string_interner.dead_entries,
            string_interner_capacity: string_interner.slot_capacity,
            string_interner_bytes: string_interner.retained_bytes,
            string_pool_bytes: string_pool.retained_bytes,
            table_pool_bytes: table_pool.retained_bytes,
            function_pool_bytes: function_pool.retained_bytes,
            cclosure_pool_bytes: cclosure_pool.retained_bytes,
            rclosure_pool_bytes: rclosure_pool.retained_bytes,
            upvalue_pool_bytes: upvalue_pool.retained_bytes,
            userdata_pool_bytes: userdata_pool.retained_bytes,
            proto_pool_bytes: proto_pool.retained_bytes,
            pool_total_slots,
            pool_live_slots,
            pool_free_slots,
            pool_page_count,
            table_cached_hash_bytes: table_alloc.hash_cached_bytes,
            table_cached_hash_blocks: table_alloc.hash_cached_blocks,
            table_cached_array_bytes: table_alloc.array_cached_bytes,
            table_cached_array_blocks: table_alloc.array_cached_blocks,
        }
    }
}
