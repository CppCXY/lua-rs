use crate::{GC, LuaValue, ObjectAllocator, lua_vm::TmKind};

/// Number of tag methods (must match TmKind::N)
const TM_N: usize = 26;

pub struct ConstString {
    // Pre-cached metamethod name strings as array indexed by TmKind discriminant.
    // Layout: tmname[TmKind::Index as usize] = "__index", etc.
    // This replaces individual named fields for O(1) lookup via array index.
    pub tmname: [LuaValue; TM_N],

    // Individual aliases kept for direct access where TmKind is not available
    pub tm_pairs: LuaValue,     // "__pairs"
    pub tm_ipairs: LuaValue,    // "__ipairs"
    pub tm_name: LuaValue,      // "__name"
    pub tm_metatable: LuaValue, // "__metatable"

    // Pre-cached coroutine status strings for fast coroutine.status
    pub str_suspended: LuaValue, // "suspended"
    pub str_running: LuaValue,   // "running"
    pub str_normal: LuaValue,    // "normal"
    pub str_dead: LuaValue,      // "dead"

    // Pre-cached type name strings for type() / tostring() / math.type()
    pub str_nil: LuaValue,      // "nil"
    pub str_boolean: LuaValue,  // "boolean"
    pub str_number: LuaValue,   // "number"
    pub str_string: LuaValue,   // "string"
    pub str_table: LuaValue,    // "table"
    pub str_function: LuaValue, // "function"
    pub str_userdata: LuaValue, // "userdata"
    pub str_thread: LuaValue,   // "thread"
    pub str_true: LuaValue,     // "true"
    pub str_false: LuaValue,    // "false"
    pub str_integer: LuaValue,  // "integer" (for math.type)
    pub str_float: LuaValue,    // "float"   (for math.type)
}

impl ConstString {
    pub fn new(allocator: &mut ObjectAllocator, gc: &mut GC) -> Self {
        let nil = LuaValue::nil();
        let mut cs = Self {
            tmname: [nil; TM_N],
            tm_pairs: nil,
            tm_ipairs: nil,
            tm_name: nil,
            tm_metatable: nil,
            str_suspended: nil,
            str_running: nil,
            str_normal: nil,
            str_dead: nil,
            str_nil: nil,
            str_boolean: nil,
            str_number: nil,
            str_string: nil,
            str_table: nil,
            str_function: nil,
            str_userdata: nil,
            str_thread: nil,
            str_true: nil,
            str_false: nil,
            str_integer: nil,
            str_float: nil,
        };

        // Pre-create all metamethod name strings indexed by TmKind discriminant
        // (like Lua's luaT_init: G(L)->tmname[i])
        let tm_names: [&str; TM_N] = [
            "__index",    // 0  Index
            "__newindex", // 1  NewIndex
            "__gc",       // 2  Gc
            "__mode",     // 3  Mode
            "__len",      // 4  Len
            "__eq",       // 5  Eq
            "__add",      // 6  Add
            "__sub",      // 7  Sub
            "__mul",      // 8  Mul
            "__mod",      // 9  Mod
            "__pow",      // 10 Pow
            "__div",      // 11 Div
            "__idiv",     // 12 IDiv
            "__band",     // 13 Band
            "__bor",      // 14 Bor
            "__bxor",     // 15 Bxor
            "__shl",      // 16 Shl
            "__shr",      // 17 Shr
            "__unm",      // 18 Unm
            "__bnot",     // 19 Bnot
            "__lt",       // 20 Lt
            "__le",       // 21 Le
            "__concat",   // 22 Concat
            "__call",     // 23 Call
            "__close",    // 24 Close
            "__tostring", // 25 ToString
        ];
        for (i, name) in tm_names.iter().enumerate() {
            cs.tmname[i] = allocator.create_string(gc, name).unwrap();
        }

        // Extra metamethod-like strings (not indexed by TmKind)
        cs.tm_pairs = allocator.create_string(gc, "__pairs").unwrap();
        cs.tm_ipairs = allocator.create_string(gc, "__ipairs").unwrap();
        cs.tm_name = allocator.create_string(gc, "__name").unwrap();
        cs.tm_metatable = allocator.create_string(gc, "__metatable").unwrap();
        // Pre-create coroutine status strings
        cs.str_suspended = allocator.create_string(gc, "suspended").unwrap();
        cs.str_running = allocator.create_string(gc, "running").unwrap();
        cs.str_normal = allocator.create_string(gc, "normal").unwrap();
        cs.str_dead = allocator.create_string(gc, "dead").unwrap();

        // Pre-create type name strings (for type(), tostring(), math.type())
        cs.str_nil = allocator.create_string(gc, "nil").unwrap();
        cs.str_boolean = allocator.create_string(gc, "boolean").unwrap();
        cs.str_number = allocator.create_string(gc, "number").unwrap();
        cs.str_string = allocator.create_string(gc, "string").unwrap();
        cs.str_table = allocator.create_string(gc, "table").unwrap();
        cs.str_function = allocator.create_string(gc, "function").unwrap();
        cs.str_userdata = allocator.create_string(gc, "userdata").unwrap();
        cs.str_thread = allocator.create_string(gc, "thread").unwrap();
        cs.str_true = allocator.create_string(gc, "true").unwrap();
        cs.str_false = allocator.create_string(gc, "false").unwrap();
        cs.str_integer = allocator.create_string(gc, "integer").unwrap();
        cs.str_float = allocator.create_string(gc, "float").unwrap();

        // Fix all metamethod name strings — they should never be collected
        // (like Lua's luaC_fix in luaT_init)
        for i in 0..TM_N {
            gc.fixed(cs.tmname[i].as_gc_ptr().unwrap());
        }
        gc.fixed(cs.tm_pairs.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_ipairs.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_name.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_metatable.as_gc_ptr().unwrap());
        gc.fixed(cs.str_suspended.as_gc_ptr().unwrap());
        gc.fixed(cs.str_running.as_gc_ptr().unwrap());
        gc.fixed(cs.str_normal.as_gc_ptr().unwrap());
        gc.fixed(cs.str_dead.as_gc_ptr().unwrap());
        gc.fixed(cs.str_nil.as_gc_ptr().unwrap());
        gc.fixed(cs.str_boolean.as_gc_ptr().unwrap());
        gc.fixed(cs.str_number.as_gc_ptr().unwrap());
        gc.fixed(cs.str_string.as_gc_ptr().unwrap());
        gc.fixed(cs.str_table.as_gc_ptr().unwrap());
        gc.fixed(cs.str_function.as_gc_ptr().unwrap());
        gc.fixed(cs.str_userdata.as_gc_ptr().unwrap());
        gc.fixed(cs.str_thread.as_gc_ptr().unwrap());
        gc.fixed(cs.str_true.as_gc_ptr().unwrap());
        gc.fixed(cs.str_false.as_gc_ptr().unwrap());
        gc.fixed(cs.str_integer.as_gc_ptr().unwrap());
        gc.fixed(cs.str_float.as_gc_ptr().unwrap());

        gc.tm_gc = cs.tmname[TmKind::Gc as usize];
        gc.tm_mode = cs.tmname[TmKind::Mode as usize];
        cs
    }

    /// Get pre-cached metamethod name string by TmKind enum value — O(1) array index.
    /// This is the fast path for metamethod lookup in hot code.
    /// Equivalent to C Lua's `G(L)->tmname[event]`.
    #[inline(always)]
    pub fn get_tm_value(&self, tm: TmKind) -> LuaValue {
        unsafe { *self.tmname.get_unchecked(tm as usize) }
    }
}
