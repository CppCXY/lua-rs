use crate::{GC, LuaValue, ObjectAllocator, lua_vm::TmKind};

pub struct ConstString {
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

impl ConstString {
    pub fn new(allocator: &mut ObjectAllocator, gc: &mut GC) -> Self {
        let mut cs = Self {
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
        // Use current_white = 0 for bootstrap (these will be fixed immediately after)
        cs.tm_index = allocator.create_string(gc, "__index");
        cs.tm_newindex = allocator.create_string(gc, "__newindex");
        cs.tm_call = allocator.create_string(gc, "__call");
        cs.tm_tostring = allocator.create_string(gc, "__tostring");
        cs.tm_len = allocator.create_string(gc, "__len");
        cs.tm_pairs = allocator.create_string(gc, "__pairs");
        cs.tm_ipairs = allocator.create_string(gc, "__ipairs");
        cs.tm_gc = allocator.create_string(gc, "__gc");
        cs.tm_close = allocator.create_string(gc, "__close");
        cs.tm_mode = allocator.create_string(gc, "__mode");
        cs.tm_name = allocator.create_string(gc, "__name");
        cs.tm_eq = allocator.create_string(gc, "__eq");
        cs.tm_lt = allocator.create_string(gc, "__lt");
        cs.tm_le = allocator.create_string(gc, "__le");
        cs.tm_add = allocator.create_string(gc, "__add");
        cs.tm_sub = allocator.create_string(gc, "__sub");
        cs.tm_mul = allocator.create_string(gc, "__mul");
        cs.tm_div = allocator.create_string(gc, "__div");
        cs.tm_mod = allocator.create_string(gc, "__mod");
        cs.tm_pow = allocator.create_string(gc, "__pow");
        cs.tm_unm = allocator.create_string(gc, "__unm");
        cs.tm_idiv = allocator.create_string(gc, "__idiv");
        cs.tm_band = allocator.create_string(gc, "__band");
        cs.tm_bor = allocator.create_string(gc, "__bor");
        cs.tm_bxor = allocator.create_string(gc, "__bxor");
        cs.tm_bnot = allocator.create_string(gc, "__bnot");
        cs.tm_shl = allocator.create_string(gc, "__shl");
        cs.tm_shr = allocator.create_string(gc, "__shr");
        cs.tm_concat = allocator.create_string(gc, "__concat");
        cs.tm_metatable = allocator.create_string(gc, "__metatable");
        // Pre-create coroutine status strings
        cs.str_suspended = allocator.create_string(gc, "suspended");
        cs.str_running = allocator.create_string(gc, "running");
        cs.str_normal = allocator.create_string(gc, "normal");
        cs.str_dead = allocator.create_string(gc, "dead");

        // Fix all metamethod name strings - they should never be collected
        // (like Lua's luaC_fix in luaT_init)
        gc.fixed(cs.tm_index.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_newindex.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_call.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_tostring.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_len.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_pairs.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_ipairs.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_gc.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_close.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_mode.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_name.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_eq.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_lt.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_le.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_add.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_sub.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_mul.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_div.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_mod.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_pow.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_unm.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_idiv.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_band.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_bor.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_bxor.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_bnot.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_shl.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_shr.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_concat.as_gc_ptr().unwrap());
        gc.fixed(cs.tm_metatable.as_gc_ptr().unwrap());
        gc.fixed(cs.str_suspended.as_gc_ptr().unwrap());
        gc.fixed(cs.str_running.as_gc_ptr().unwrap());
        gc.fixed(cs.str_normal.as_gc_ptr().unwrap());
        gc.fixed(cs.str_dead.as_gc_ptr().unwrap());

        gc.tm_gc = cs.tm_gc.clone();
        gc.tm_mode = cs.tm_mode.clone();
        cs
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
            TmKind::ToString => self.tm_tostring,
            _ => self.tm_index, // Fallback to __index
        }
    }
}
