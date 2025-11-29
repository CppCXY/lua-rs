use crate::LuaValue;

/// LuaCallFrame - 精简版，仿照 Lua C 的 CallInfo
///
/// 关键字段：
/// - function_value: 函数值（包含ID）
/// - code_ptr: 直接指向指令数组，热路径优化
/// - constants_ptr: 直接指向常量数组，热路径优化
/// - base_ptr: 寄存器栈基址  
/// - top: 栈顶（用于参数传递）
/// - pc: 程序计数器
/// - callstatus: 调用状态标志位（仿照 Lua 的 CIST_* 标志）
/// - nresults: 期望返回数
/// - result_reg: 返回值写入的寄存器位置
/// - vararg_start: vararg 参数在栈上的起始位置（绝对索引）
/// - vararg_count: vararg 参数数量
///
/// 内存布局（72 bytes）:
/// - 16 bytes: function_value
/// - 8 bytes: code_ptr  
/// - 8 bytes: constants_ptr
/// - 8 bytes: base_ptr
/// - 8 bytes: top
/// - 8 bytes: pc
/// - 4 bytes: result_reg (u32)
/// - 4 bytes: vararg_start (u32) - vararg 起始位置
/// - 2 bytes: nresults (i16)
/// - 2 bytes: vararg_count (u16)
/// - 1 byte: callstatus
/// - 3 bytes: padding
pub struct LuaCallFrame {
    pub function_value: LuaValue,     // 16 bytes
    pub code_ptr: *const u32,         // 8 bytes - 直接指向指令数组
    pub constants_ptr: *const LuaValue, // 8 bytes - 直接指向常量数组
    pub base_ptr: usize,              // 8 bytes - 寄存器栈基址
    pub top: usize,                   // 8 bytes - 栈顶
    pub pc: usize,                    // 8 bytes - 程序计数器
    result_reg: u32,                  // 4 bytes - 返回值写入位置
    vararg_start: u32,                // 4 bytes - vararg 起始位置（绝对索引）
    nresults: i16,                    // 2 bytes - 期望返回数 (-1 = LUA_MULTRET)
    vararg_count: u16,                // 2 bytes - vararg 参数数量
    pub callstatus: u8,               // 1 byte - 调用状态标志
    _pad: [u8; 3],                    // 3 bytes - 对齐到 8 字节边界
}

// CallStatus flags (仿照 Lua 的 CIST_* 标志)
pub const CIST_LUA: u8 = 1 << 0;       // 是Lua函数
pub const CIST_FRESH: u8 = 1 << 1;     // 新调用，返回时应停止执行
pub const CIST_YPCALL: u8 = 1 << 2;    // 是 pcall（protected call）
pub const CIST_TAIL: u8 = 1 << 3;      // 尾调用

// 特殊值
#[allow(dead_code)]
pub const LUA_MULTRET: i16 = -1;

impl LuaCallFrame {
    #[inline(always)]
    pub fn new_lua_function(
        function_value: LuaValue,
        code_ptr: *const u32,
        constants_ptr: *const LuaValue,
        base_ptr: usize,
        top: usize,
        result_reg: usize,
        nresults: i16,
    ) -> Self {
        LuaCallFrame {
            function_value,
            code_ptr,
            constants_ptr,
            base_ptr,
            top,
            pc: 0,
            result_reg: result_reg as u32,
            vararg_start: 0,
            nresults,
            vararg_count: 0,
            callstatus: CIST_LUA,
            _pad: [0; 3],
        }
    }

    #[inline(always)]
    pub fn new_c_function(base_ptr: usize, top: usize) -> Self {
        LuaCallFrame {
            function_value: LuaValue::nil(),
            code_ptr: std::ptr::null(),
            constants_ptr: std::ptr::null(),
            base_ptr,
            top,
            pc: 0,
            result_reg: 0,
            vararg_start: 0,
            nresults: 0,
            vararg_count: 0,
            callstatus: 0, // C function, not CIST_LUA
            _pad: [0; 3],
        }
    }

    /// Set vararg information for this frame
    #[inline(always)]
    pub fn set_vararg(&mut self, start: usize, count: usize) {
        self.vararg_start = start as u32;
        self.vararg_count = count as u16;
    }

    /// Get vararg start position (absolute stack index)
    #[inline(always)]
    pub fn get_vararg_start(&self) -> usize {
        self.vararg_start as usize
    }

    /// Get vararg count
    #[inline(always)]
    pub fn get_vararg_count(&self) -> usize {
        self.vararg_count as usize
    }

    #[inline(always)]
    pub fn is_lua(&self) -> bool {
        self.callstatus & CIST_LUA != 0
    }

    #[inline(always)]
    pub fn is_fresh(&self) -> bool {
        self.callstatus & CIST_FRESH != 0
    }

    #[inline(always)]
    pub fn set_fresh(&mut self) {
        self.callstatus |= CIST_FRESH;
    }

    #[inline(always)]
    pub fn clear_fresh(&mut self) {
        self.callstatus &= !CIST_FRESH;
    }

    #[inline(always)]
    pub fn is_protected(&self) -> bool {
        self.callstatus & CIST_YPCALL != 0
    }

    #[inline(always)]
    pub fn set_protected(&mut self, protected: bool) {
        if protected {
            self.callstatus |= CIST_YPCALL;
        } else {
            self.callstatus &= !CIST_YPCALL;
        }
    }

    #[inline(always)]
    pub fn is_tailcall(&self) -> bool {
        self.callstatus & CIST_TAIL != 0
    }

    #[inline(always)]
    pub fn set_tailcall(&mut self) {
        self.callstatus |= CIST_TAIL;
    }

    #[inline(always)]
    pub fn get_nresults(&self) -> i16 {
        self.nresults
    }

    #[inline(always)]
    pub fn set_nresults(&mut self, n: i16) {
        self.nresults = n;
    }

    #[inline(always)]
    pub fn get_result_reg(&self) -> usize {
        self.result_reg as usize
    }

    #[inline(always)]
    pub fn get_num_results(&self) -> usize {
        if self.nresults < 0 {
            usize::MAX
        } else {
            self.nresults as usize
        }
    }

    /// 获取函数 ID - 用于 ObjectPool 查找
    #[inline(always)]
    pub fn get_function_id(&self) -> Option<crate::gc::FunctionId> {
        if self.is_lua() {
            self.function_value.as_function_id()
        } else {
            None
        }
    }
}

impl std::fmt::Debug for LuaCallFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaCallFrame")
            .field("base_ptr", &self.base_ptr)
            .field("top", &self.top)
            .field("pc", &self.pc)
            .field("is_lua", &self.is_lua())
            .field("is_fresh", &self.is_fresh())
            .finish()
    }
}
