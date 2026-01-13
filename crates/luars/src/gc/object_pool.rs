// Object Pool V3 - Simplified high-performance design
//
// Key Design Principles:
// 1. All IDs are u32 indices into Vec storage
// 2. Small objects (String, Function, Upvalue) use Vec<Option<T>>
// 3. Large objects (Table, Thread) use Vec<Option<Box<T>>> to avoid copy on resize
// 4. No chunking overhead - direct Vec indexing for O(1) access
// 5. Free list for slot reuse
// 6. GC headers embedded in objects for mark-sweep

use crate::gc::gc_object::{CachedUpvalue, FunctionBody};
use crate::gc::string_interner::StringInterner;
use crate::lua_value::{Chunk, LuaUpvalue, LuaUserdata};
use crate::lua_vm::{CFunction, LuaState, SafeOption, TmKind};
use crate::{
    BinaryId, FunctionId, GcId, GcObject, GcObjectType, GcPool, GcPtrObject, LuaTable, LuaValue,
    StringId, TableId, ThreadId, Upvalue, UpvalueId, UserdataId,
};
use std::rc::Rc;

/// High-performance object pool for the Lua VM
/// - Small objects (String, Function, Upvalue) use Pool<T> with direct Vec storage
/// - Large objects (Table, Thread) use BoxPool<T> to avoid copy on resize
/// - ALL strings are interned via StringInterner for O(1) equality checks
pub struct ObjectPool {
    strings: StringInterner,    // Private - use create_string() to intern
    pub(crate) gc_pool: GcPool, // General GC pool for all objects
    // Pre-cached metamethod name StringIds (like Lua's G(L)->tmname[])
    // These are created at initialization and never collected
    // Stored as StringId to avoid repeated hash lookup in hot paths
    pub tm_index: LuaValue,     // "__index"
    pub tm_newindex: LuaValue,  // "__newindex"
    pub tm_call: LuaValue,      // "__call"
    pub tm_tostring: LuaValue,  // "__tostring"
    pub tm_len: LuaValue,       // "__len"
    pub tm_pairs: LuaValue,     // "__pairs"
    pub tm_ipairs: LuaValue,    // "__ipairs"
    pub tm_gc: LuaValue,        // "__gc"
    pub tm_close: LuaValue,     // "__close"
    pub tm_mode: LuaValue,      // "__mode"
    pub tm_name: LuaValue,      // "__name"
    pub tm_eq: LuaValue,        // "__eq"
    pub tm_lt: LuaValue,        // "__lt"
    pub tm_le: LuaValue,        // "__le"
    pub tm_add: LuaValue,       // "__add"
    pub tm_sub: LuaValue,       // "__sub"
    pub tm_mul: LuaValue,       // "__mul"
    pub tm_div: LuaValue,       // "__div"
    pub tm_mod: LuaValue,       // "__mod"
    pub tm_pow: LuaValue,       // "__pow"
    pub tm_unm: LuaValue,       // "__unm"
    pub tm_idiv: LuaValue,      // "__idiv"
    pub tm_band: LuaValue,      // "__band"
    pub tm_bor: LuaValue,       // "__bor"
    pub tm_bxor: LuaValue,      // "__bxor"
    pub tm_bnot: LuaValue,      // "__bnot"
    pub tm_shl: LuaValue,       // "__shl"
    pub tm_shr: LuaValue,       // "__shr"
    pub tm_concat: LuaValue,    // "__concat"
    pub tm_metatable: LuaValue, // "__metatable"

    // Pre-cached coroutine status strings for fast coroutine.status
    pub str_suspended: LuaValue, // "suspended"
    pub str_running: LuaValue,   // "running"
    pub str_normal: LuaValue,    // "normal"
    pub str_dead: LuaValue,      // "dead"
}

impl ObjectPool {
    pub fn new(option: SafeOption) -> Self {
        let mut pool = Self {
            strings: StringInterner::new(option.small_string_limit),
            gc_pool: GcPool::new(),
            // Placeholder values - will be initialized below
            tm_index: LuaValue::nil(),
            tm_newindex: LuaValue::nil(),
            tm_call: LuaValue::nil(),
            tm_tostring: LuaValue::nil(),
            tm_len: LuaValue::nil(),
            tm_pairs: LuaValue::nil(),
            tm_ipairs: LuaValue::nil(),
            tm_gc: LuaValue::nil(),
            tm_close: LuaValue::nil(),
            tm_mode: LuaValue::nil(),
            tm_name: LuaValue::nil(),
            tm_eq: LuaValue::nil(),
            tm_lt: LuaValue::nil(),
            tm_le: LuaValue::nil(),
            tm_add: LuaValue::nil(),
            tm_sub: LuaValue::nil(),
            tm_mul: LuaValue::nil(),
            tm_div: LuaValue::nil(),
            tm_mod: LuaValue::nil(),
            tm_pow: LuaValue::nil(),
            tm_unm: LuaValue::nil(),
            tm_idiv: LuaValue::nil(),
            tm_band: LuaValue::nil(),
            tm_bor: LuaValue::nil(),
            tm_bxor: LuaValue::nil(),
            tm_bnot: LuaValue::nil(),
            tm_shl: LuaValue::nil(),
            tm_shr: LuaValue::nil(),
            tm_concat: LuaValue::nil(),
            tm_metatable: LuaValue::nil(),
            str_suspended: LuaValue::nil(),
            str_running: LuaValue::nil(),
            str_normal: LuaValue::nil(),
            str_dead: LuaValue::nil(),
        };

        // Pre-create all metamethod name strings (like Lua's luaT_init)
        // These strings are interned and will never be collected
        pool.tm_index = pool.create_string("__index").0;
        pool.tm_newindex = pool.create_string("__newindex").0;
        pool.tm_call = pool.create_string("__call").0;
        pool.tm_tostring = pool.create_string("__tostring").0;
        pool.tm_len = pool.create_string("__len").0;
        pool.tm_pairs = pool.create_string("__pairs").0;
        pool.tm_ipairs = pool.create_string("__ipairs").0;
        pool.tm_gc = pool.create_string("__gc").0;
        pool.tm_close = pool.create_string("__close").0;
        pool.tm_mode = pool.create_string("__mode").0;
        pool.tm_name = pool.create_string("__name").0;
        pool.tm_eq = pool.create_string("__eq").0;
        pool.tm_lt = pool.create_string("__lt").0;
        pool.tm_le = pool.create_string("__le").0;
        pool.tm_add = pool.create_string("__add").0;
        pool.tm_sub = pool.create_string("__sub").0;
        pool.tm_mul = pool.create_string("__mul").0;
        pool.tm_div = pool.create_string("__div").0;
        pool.tm_mod = pool.create_string("__mod").0;
        pool.tm_pow = pool.create_string("__pow").0;
        pool.tm_unm = pool.create_string("__unm").0;
        pool.tm_idiv = pool.create_string("__idiv").0;
        pool.tm_band = pool.create_string("__band").0;
        pool.tm_bor = pool.create_string("__bor").0;
        pool.tm_bxor = pool.create_string("__bxor").0;
        pool.tm_bnot = pool.create_string("__bnot").0;
        pool.tm_shl = pool.create_string("__shl").0;
        pool.tm_shr = pool.create_string("__shr").0;
        pool.tm_concat = pool.create_string("__concat").0;
        pool.tm_metatable = pool.create_string("__metatable").0;

        // Pre-create coroutine status strings
        pool.str_suspended = pool.create_string("suspended").0;
        pool.str_running = pool.create_string("running").0;
        pool.str_normal = pool.create_string("normal").0;
        pool.str_dead = pool.create_string("dead").0;

        // Fix all metamethod name strings - they should never be collected
        // (like Lua's luaC_fix in luaT_init)
        pool.fix_gc_object(pool.tm_index.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_newindex.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_call.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_tostring.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_len.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_pairs.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_ipairs.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_gc.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_close.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_mode.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_name.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_eq.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_lt.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_le.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_add.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_sub.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_mul.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_div.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_mod.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_pow.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_unm.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_idiv.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_band.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_bor.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_bxor.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_bnot.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_shl.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_shr.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_concat.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.tm_metatable.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.str_suspended.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.str_running.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.str_normal.as_string_id().unwrap().into());
        pool.fix_gc_object(pool.str_dead.as_string_id().unwrap().into());

        pool
    }

    pub fn get_short_string_limit(&self) -> usize {
        self.strings.small_string_limit
    }

    /// Get pre-cached metamethod StringId by TM enum value
    /// This is the fast path for metamethod lookup in hot code
    /// TMS enum from ltm.h:
    /// TM_INDEX=0, TM_NEWINDEX=1, TM_GC=2, TM_MODE=3, TM_LEN=4, TM_EQ=5,
    /// TM_ADD=6, TM_SUB=7, TM_MUL=8, TM_MOD=9, TM_POW=10, TM_DIV=11,
    /// TM_IDIV=12, TM_BAND=13, TM_BOR=14, TM_BXOR=15, TM_SHL=16, TM_SHR=17,
    /// TM_UNM=18, TM_BNOT=19, TM_LT=20, TM_LE=21, TM_CONCAT=22, TM_CALL=23
    #[inline]
    pub fn get_tm_value(&self, tm: TmKind) -> LuaValue {
        match tm {
            TmKind::Index => self.tm_index,
            TmKind::NewIndex => self.tm_newindex,
            TmKind::Gc => self.tm_gc,
            TmKind::Mode => self.tm_mode,
            TmKind::Len => self.tm_len,
            TmKind::Eq => self.tm_eq,
            TmKind::Add => self.tm_add,
            TmKind::Sub => self.tm_sub,
            TmKind::Mul => self.tm_mul,
            TmKind::Mod => self.tm_mod,
            TmKind::Pow => self.tm_pow,
            TmKind::Div => self.tm_div,
            TmKind::IDiv => self.tm_idiv,
            TmKind::Band => self.tm_band,
            TmKind::Bor => self.tm_bor,
            TmKind::Bxor => self.tm_bxor,
            TmKind::Shl => self.tm_shl,
            TmKind::Shr => self.tm_shr,
            TmKind::Unm => self.tm_unm,
            TmKind::Bnot => self.tm_bnot,
            TmKind::Lt => self.tm_lt,
            TmKind::Le => self.tm_le,
            TmKind::Concat => self.tm_concat,
            TmKind::Call => self.tm_call,
            TmKind::Close => self.tm_close,
            _ => self.tm_index, // Fallback to __index
        }
    }

    #[inline]
    pub fn get_tm_value_by_str(&self, tm_str: &str) -> LuaValue {
        match tm_str {
            "__index" => self.tm_index,
            "__newindex" => self.tm_newindex,
            "__gc" => self.tm_gc,
            "__mode" => self.tm_mode,
            "__len" => self.tm_len,
            "__eq" => self.tm_eq,
            "__add" => self.tm_add,
            "__sub" => self.tm_sub,
            "__mul" => self.tm_mul,
            "__mod" => self.tm_mod,
            "__pow" => self.tm_pow,
            "__div" => self.tm_div,
            "__idiv" => self.tm_idiv,
            "__band" => self.tm_band,
            "__bor" => self.tm_bor,
            "__bxor" => self.tm_bxor,
            "__shl" => self.tm_shl,
            "__shr" => self.tm_shr,
            "__unm" => self.tm_unm,
            "__bnot" => self.tm_bnot,
            "__lt" => self.tm_lt,
            "__le" => self.tm_le,
            "__concat" => self.tm_concat,
            "__call" => self.tm_call,
            "__close" => self.tm_close,
            "__tostring" => self.tm_tostring,
            _ => self.tm_index, // Fallback to __index
        }
    }

    // ==================== String Operations ====================

    /// Create or intern a string (Lua-style with proper hash collision handling)
    /// Returns (StringId, is_new) where is_new indicates if a new string was created
    #[inline]
    /// Create string (COMPLETE INTERNING - all strings)
    /// Returns (StringId, is_new) where is_new indicates if a new string was created
    pub fn create_string(&mut self, s: &str) -> (LuaValue, bool) {
        self.strings.intern(s, &mut self.gc_pool)
    }

    /// Create string from owned String (avoids clone if already interned)
    /// Returns (StringId, is_new) where is_new indicates if a new string was created
    pub fn create_string_owned(&mut self, s: String) -> (LuaValue, bool) {
        self.strings.intern(&s, &mut self.gc_pool)
    }

    pub fn get_string_value(&self, id: StringId) -> Option<LuaValue> {
        let gc_obj = self.gc_pool.get(id.0)?;
        let ptr = match &gc_obj.ptr {
            GcPtrObject::String(s) => s.as_ref() as *const String,
            _ => return None,
        };
        Some(LuaValue::string(id, ptr))
    }

    /// Create a binary value from Vec<u8>
    #[inline]
    pub fn create_binary(&mut self, data: Vec<u8>) -> LuaValue {
        let gc_binary = GcObject::new(GcPtrObject::Binary(Box::new(data)));
        let ptr = gc_binary.ptr.as_binary_ptr().unwrap();
        let id = self.gc_pool.alloc(gc_binary);
        let binary_id = BinaryId(id);

        LuaValue::binary(binary_id, ptr)
    }

    #[inline(always)]
    pub fn get_binary(&self, id: BinaryId) -> Option<&[u8]> {
        self.gc_pool.get(id.0).and_then(|go| match &go.ptr {
            GcPtrObject::Binary(b) => Some(b.as_ref().as_slice()),
            _ => None,
        })
    }

    #[inline(always)]
    pub fn get_binary_value(&self, id: BinaryId) -> Option<LuaValue> {
        let gc_obj = self.gc_pool.get(id.0)?;
        let ptr = match &gc_obj.ptr {
            GcPtrObject::Binary(b) => b.as_ref() as *const Vec<u8>,
            _ => return None,
        };
        Some(LuaValue::binary(id, ptr))
    }

    #[inline(always)]
    pub fn get_string(&self, id: StringId) -> Option<&str> {
        self.gc_pool.get(id.0).and_then(|go| match &go.ptr {
            GcPtrObject::String(s) => Some(s.as_ref().as_str()),
            _ => None,
        })
    }

    #[inline(always)]
    pub fn contains_string(&self, id: StringId) -> bool {
        self.gc_pool.get(id.0).is_some()
    }

    /// Create a substring from an existing string (optimized for string.sub)
    /// Returns the original string ID if the range covers the entire string.
    /// With complete interning, substrings are automatically deduplicated.
    #[inline]
    pub fn create_substring(&mut self, s_value: LuaValue, start: usize, end: usize) -> LuaValue {
        let string = match s_value.as_str() {
            Some(s) => s,
            None => return self.create_string("").0,
        };
        // Extract substring info first
        let substring = {
            // Clamp indices
            let start = start.min(string.len());
            let end = end.min(string.len());

            if start >= end {
                return self.create_string("").0;
            }

            // Fast path: return original if full range
            if start == 0 && end == string.len() {
                return s_value;
            }

            // Copy substring to avoid borrowing issue
            &string[start..end]
        };

        // Intern the substring - will be deduplicated if it already exists
        self.create_string(substring).0
    }

    /// Mark a string as fixed (never collected) - like Lua's luaC_fix()
    /// Used for metamethod names and other permanent strings
    #[inline]
    pub fn fix_gc_object(&mut self, id: GcId) {
        if let Some(go) = self.gc_pool.get_mut(id.index()) {
            go.header.set_fixed();
            go.header.make_black(); // Always considered marked
        }
    }
    // ==================== Table Operations ====================

    #[inline]
    pub fn create_table(&mut self, array_size: usize, hash_size: usize) -> LuaValue {
        let gc_table = GcObject::new(GcPtrObject::Table(Box::new(LuaTable::new(
            array_size as u32,
            hash_size as u32,
        ))));
        let ptr = gc_table.ptr.as_table_ptr().unwrap();
        let id = self.gc_pool.alloc(gc_table);
        let table_id = TableId(id);
        LuaValue::table(table_id, ptr)
    }

    #[inline(always)]
    pub fn get_table(&self, id: TableId) -> Option<&LuaTable> {
        let table = self.gc_pool.get(id.0)?;
        match &table.ptr {
            GcPtrObject::Table(t) => Some(t.as_ref()),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn get_table_value(&self, id: TableId) -> Option<LuaValue> {
        let table = self.gc_pool.get(id.0)?;
        let ptr = match &table.ptr {
            GcPtrObject::Table(t) => t.as_ref() as *const LuaTable,
            _ => return None,
        };
        Some(LuaValue::table(id, ptr))
    }

    #[inline(always)]
    pub fn get_table_mut(&mut self, id: TableId) -> Option<&mut LuaTable> {
        let table = self.gc_pool.get_mut(id.0)?;
        match &mut table.ptr {
            GcPtrObject::Table(t) => Some(t.as_mut()),
            _ => None,
        }
    }

    // ==================== Function Operations ====================

    /// Create a Lua function (closure with bytecode chunk)
    /// Now caches upvalue pointers for direct access
    #[inline]
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalue_ids: Vec<UpvalueId>) -> LuaValue {
        // Build cached upvalues with direct pointers
        let mut upvalues: Vec<CachedUpvalue> = vec![];
        for id in upvalue_ids {
            if let Some(uv) = self.gc_pool.get(id.0) {
                let ptr = uv.ptr.as_upvalue_ptr().unwrap();
                upvalues.push(CachedUpvalue::new(id, ptr));
            }
        }

        let gc_func = GcObject::new(GcPtrObject::Function(Box::new(FunctionBody::Lua(
            chunk, upvalues,
        ))));
        let ptr = gc_func.ptr.as_function_ptr().unwrap();
        let id = self.gc_pool.alloc(gc_func);
        let func_id = FunctionId(id);
        LuaValue::function(func_id, ptr)
    }

    /// Create a C closure (native function with upvalues)
    /// Now caches upvalue pointers for direct access
    #[inline]
    pub fn create_c_closure(&mut self, func: CFunction, upvalue_ids: Vec<UpvalueId>) -> LuaValue {
        // Build cached upvalues with direct pointers
        let mut upvalues: Vec<CachedUpvalue> = vec![];
        for id in upvalue_ids {
            if let Some(uv) = self.gc_pool.get(id.0) {
                let ptr = uv.ptr.as_upvalue_ptr().unwrap();
                upvalues.push(CachedUpvalue::new(id, ptr));
            }
        }

        let gc_func = GcObject::new(GcPtrObject::Function(Box::new(FunctionBody::CClosure(
            func, upvalues,
        ))));
        let ptr = gc_func.ptr.as_function_ptr().unwrap();
        let id = self.gc_pool.alloc(gc_func);
        let func_id = FunctionId(id);
        LuaValue::function(func_id, ptr)
    }

    // ==================== Upvalue Operations ====================

    /// Create an open upvalue pointing to a stack location
    #[inline]
    pub fn create_upvalue_open(&mut self, stack_index: usize) -> UpvalueId {
        let upvalue = Upvalue::Open(stack_index);

        let gc_uv = GcObject::new(GcPtrObject::Upvalue(Box::new(upvalue)));
        UpvalueId(self.gc_pool.alloc(gc_uv))
    }

    /// Create a closed upvalue with a value
    #[inline]
    pub fn create_upvalue_closed(&mut self, value: LuaValue) -> UpvalueId {
        let upvalue = Upvalue::Closed(value);
        let gc_uv = GcObject::new(GcPtrObject::Upvalue(Box::new(upvalue)));
        UpvalueId(self.gc_pool.alloc(gc_uv))
    }

    #[inline(always)]
    pub(crate) fn get_upvalue(&self, id: UpvalueId) -> Option<&Upvalue> {
        let gc_obj = self.gc_pool.get(id.0)?;
        match &gc_obj.ptr {
            GcPtrObject::Upvalue(upvalue) => Some(upvalue.as_ref()),
            _ => None,
        }
    }

    #[inline(always)]
    pub(crate) fn get_upvalue_mut(&mut self, id: UpvalueId) -> Option<&mut Upvalue> {
        let gc_obj = self.gc_pool.get_mut(id.0)?;
        match &mut gc_obj.ptr {
            GcPtrObject::Upvalue(upvalue) => Some(upvalue.as_mut()),
            _ => None,
        }
    }

    /// Create upvalue from LuaUpvalue
    pub fn create_upvalue(&mut self, upvalue: Rc<LuaUpvalue>) -> UpvalueId {
        // Check if open and get stack index
        if upvalue.is_open() {
            self.create_upvalue_open(upvalue.get_stack_index().unwrap_or(0))
        } else {
            self.create_upvalue_closed(upvalue.get_closed_value().unwrap_or(LuaValue::nil()))
        }
    }

    // ==================== Userdata Operations ====================

    #[inline]
    pub fn create_userdata(&mut self, userdata: LuaUserdata) -> LuaValue {
        let gc_userdata = GcObject::new(GcPtrObject::Userdata(Box::new(userdata)));
        let ptr = gc_userdata.ptr.as_userdata_ptr().unwrap();
        let id = UserdataId(self.gc_pool.alloc(gc_userdata));
        LuaValue::userdata(id, ptr)
    }

    #[inline(always)]
    pub fn get_userdata_mut(&mut self, id: UserdataId) -> Option<&mut LuaUserdata> {
        self.gc_pool
            .get_mut(id.0)
            .and_then(|gc_obj| match &mut gc_obj.ptr {
                GcPtrObject::Userdata(userdata) => Some(userdata.as_mut()),
                _ => None,
            })
    }

    // ==================== Thread Operations ====================

    #[inline]
    pub fn create_thread(&mut self, thread: LuaState) -> LuaValue {
        let gc_thread = GcObject::new(GcPtrObject::Thread(Box::new(thread)));
        let ptr = gc_thread.ptr.as_thread_ptr().unwrap();
        let id = ThreadId(self.gc_pool.alloc(gc_thread));
        let l = self.get_thread_mut(id).unwrap();
        l.set_thread_id(id);

        LuaValue::thread(id, ptr)
    }

    #[inline(always)]
    pub fn get_thread_value(&self, id: ThreadId) -> Option<LuaValue> {
        let gc = self.gc_pool.get(id.0)?;
        let ptr = match &gc.ptr {
            GcPtrObject::Thread(t) => t.as_ref() as *const LuaState,
            _ => return None,
        };
        Some(LuaValue::thread(id, ptr))
    }

    #[inline(always)]
    pub fn get_thread_mut(&mut self, id: ThreadId) -> Option<&mut LuaState> {
        self.gc_pool
            .get_mut(id.0)
            .and_then(|gc_obj| match &mut gc_obj.ptr {
                GcPtrObject::Thread(t) => Some(t.as_mut()),
                _ => None,
            })
    }
    // ==================== GC Support ====================

    /// Clear all mark bits before GC mark phase (make all objects white)
    pub fn clear_marks(&mut self) {
        for (_, gs) in self.gc_pool.iter_mut() {
            gs.header.make_white(0);
        }
    }

    pub fn shrink_to_fit(&mut self) {
        // StringInterner manages its own internal structures
        self.gc_pool.shrink_to_fit();
    }

    pub fn get(&self, id: GcId) -> Option<&GcObject> {
        self.gc_pool.get(id.index())
    }

    pub fn get_mut(&mut self, id: GcId) -> Option<&mut GcObject> {
        self.gc_pool.get_mut(id.index())
    }

    #[inline]
    pub fn remove(&mut self, id: GcId) {
        match id.gc_type() {
            GcObjectType::String => {
                if let Some(s_object) = self.gc_pool.get(id.index())
                    && let GcPtrObject::String(s) = &s_object.ptr
                {
                    // Remove from string interner first
                    self.strings.remove_dead_intern(StringId(id.index()), &s);
                    self.gc_pool.free(id.index());
                }
            }
            _ => {
                self.gc_pool.free(id.index());
            }
        }
    }
}

impl Default for ObjectPool {
    fn default() -> Self {
        Self::new(SafeOption::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_interning() {
        let mut pool = ObjectPool::default();

        let v1 = pool.create_string("hello").0;
        let v2 = pool.create_string("hello").0;

        // Same string should return same ID
        assert_eq!(v1, v2);
        let v3 = pool.create_string("world").0;
        assert_ne!(v1, v3);

        // Verify content
        assert_eq!(v1.as_str(), Some("hello"));
        assert_eq!(v3.as_str(), Some("world"));
    }

    #[test]
    fn test_table_operations() {
        let mut pool = ObjectPool::default();

        let table_value = pool.create_table(4, 4);
        let table_id = table_value.as_table_id().unwrap();
        // Modify table
        if let Some(table) = pool.get_table_mut(table_id) {
            table.raw_set(&LuaValue::integer(1), LuaValue::integer(42));
        }

        // Read back
        if let Some(table) = pool.get_table(table_id) {
            assert!(table.raw_get(&LuaValue::integer(1)) == Some(LuaValue::integer(42)));
        }
    }

    #[test]
    fn test_object_ids_size() {
        // Verify all IDs are compact 4 bytes
        assert_eq!(std::mem::size_of::<StringId>(), 4);
        assert_eq!(std::mem::size_of::<TableId>(), 4);
        assert_eq!(std::mem::size_of::<FunctionId>(), 4);
        assert_eq!(std::mem::size_of::<UpvalueId>(), 4);
        assert_eq!(std::mem::size_of::<UserdataId>(), 4);
        assert_eq!(std::mem::size_of::<ThreadId>(), 4);
    }

    #[test]
    fn test_string_interning_many_strings() {
        // Test that many different strings with potential hash collisions
        // are all stored correctly
        let mut pool = ObjectPool::default();
        let mut ids = Vec::new();

        // Create 1000 different strings
        for i in 0..1000 {
            let s = format!("string_{}", i);
            let id = pool.create_string(&s).0;
            ids.push((s, id));
        }

        // Verify all strings are stored correctly
        for (s, id) in &ids {
            let stored = id.as_str();
            assert_eq!(
                stored,
                Some(s.as_str()),
                "String '{}' not stored correctly",
                s
            );
        }

        // Verify interning works - same string should return same ID
        for (s, id) in &ids {
            let id2 = pool.create_string(s).0;
            assert_eq!(*id, id2, "Interning failed for '{}'", s);
        }
    }

    #[test]
    fn test_string_interning_similar_strings() {
        // Test strings that might have similar hashes
        let mut pool = ObjectPool::default();

        let strings = vec![
            "a", "b", "c", "aa", "ab", "ba", "bb", "aaa", "aab", "aba", "abb", "baa", "bab", "bba",
            "bbb", "test", "Test", "TEST", "tEsT", "hello", "Hello", "HELLO", "hElLo",
        ];

        let mut ids = Vec::new();
        for s in &strings {
            ids.push(pool.create_string(s).0);
        }

        // All IDs should be unique (different strings)
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(
                    ids[i], ids[j],
                    "Different strings '{}' and '{}' got same ID",
                    strings[i], strings[j]
                );
            }
        }

        // Verify content
        for (i, s) in strings.iter().enumerate() {
            assert_eq!(ids[i].as_str(), Some(*s));
        }
    }
}
