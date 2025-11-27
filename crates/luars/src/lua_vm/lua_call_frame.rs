use crate::LuaValue;

/// ULTRA-OPTIMIZED LuaCallFrame - 仿照原生 Lua 的 CallInfo 设计
///
/// 关键优化：直接缓存 code 指针，避免每次循环都要解引用 function → chunk → code
///
/// 内存布局（72 bytes）：
/// - 16 bytes: function_value (LuaValue - 包含函数ID)
/// - 8 bytes: code_ptr (直接指向指令数组 - HOT PATH!)
/// - 40 bytes: base_ptr + top + pc + frame_id + vararg_start (5×usize)
/// - 6 bytes: result_reg + num_results + vararg_count (3×u16)
/// - 1 byte: flags
/// - 1 byte: padding
pub struct LuaCallFrame {
    pub function_value: LuaValue, // 16 bytes - 包含 ID + 指针
    pub code_ptr: *const u32,     // 8 bytes - HOT PATH: 直接指向指令数组!
    pub base_ptr: usize,          // 8 bytes - 寄存器栈基址
    pub top: usize,               // 8 bytes - 栈顶
    pub pc: usize,                // 8 bytes - 程序计数器
    pub frame_id: usize,          // 8 bytes - 帧ID
    pub vararg_start: usize,      // 8 bytes - 可变参数起始位置
    pub result_reg: u16,          // 2 bytes - 结果寄存器索引
    pub num_results: u16,         // 2 bytes - 期望返回数
    pub vararg_count: u16,        // 2 bytes - 可变参数数量
    flags: u8,                    // 1 byte - 标志位
    _padding: u8,                 // 1 byte - 对齐
                                  // Total: 72 bytes
}

// Flag bits
const FLAG_IS_LUA: u8 = 1 << 0;
const FLAG_IS_PROTECTED: u8 = 1 << 1;

// 特殊值：num_results = 0xFFFF 表示接受多个返回值
const NUM_RESULTS_MULTIPLE: u16 = 0xFFFF;

impl LuaCallFrame {
    #[inline]
    pub fn new_lua_function(
        frame_id: usize,
        function_value: LuaValue,
        code_ptr: *const u32, // 新增：直接传入 code 指针
        base_ptr: usize,
        max_stack_size: usize,
        result_reg: usize,
        num_results: usize,
    ) -> Self {
        LuaCallFrame {
            function_value,
            code_ptr,
            base_ptr,
            top: max_stack_size,
            pc: 0,
            frame_id,
            result_reg: result_reg as u16,
            num_results: if num_results == usize::MAX {
                NUM_RESULTS_MULTIPLE
            } else {
                num_results.min(65534) as u16
            },
            vararg_start: 0,
            vararg_count: 0,
            flags: FLAG_IS_LUA,
            _padding: 0,
        }
    }

    #[inline]
    pub fn new_c_function(
        frame_id: usize,
        parent_function_value: LuaValue,
        parent_pc: usize,
        base_ptr: usize,
        num_args: usize,
    ) -> Self {
        LuaCallFrame {
            function_value: parent_function_value,
            code_ptr: std::ptr::null(), // C 函数没有 code
            base_ptr,
            top: num_args,
            pc: parent_pc,
            frame_id,
            result_reg: 0,
            num_results: 0,
            vararg_start: 0,
            vararg_count: 0,
            flags: 0, // C function
            _padding: 0,
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

    #[inline(always)]
    pub fn is_lua(&self) -> bool {
        self.flags & FLAG_IS_LUA != 0
    }

    #[inline(always)]
    pub fn is_protected(&self) -> bool {
        self.flags & FLAG_IS_PROTECTED != 0
    }

    #[inline(always)]
    pub fn set_protected(&mut self, protected: bool) {
        if protected {
            self.flags |= FLAG_IS_PROTECTED;
        } else {
            self.flags &= !FLAG_IS_PROTECTED;
        }
    }

    /// 获取 FunctionId (需要时才调用)
    // Note: already defined above, this is a duplicate - keeping the earlier definition
    // #[inline(always)]
    // pub fn get_function_id(&self) -> Option<FunctionId> {
    //     self.function_value.as_function_id()
    // }

    // === 类型转换辅助方法（零开销内联）===

    #[inline(always)]
    pub fn get_result_reg(&self) -> usize {
        self.result_reg as usize
    }

    #[inline(always)]
    pub fn get_num_results(&self) -> usize {
        if self.num_results == NUM_RESULTS_MULTIPLE {
            usize::MAX
        } else {
            self.num_results as usize
        }
    }

    #[inline(always)]
    pub fn get_vararg_count(&self) -> usize {
        self.vararg_count as usize
    }

    #[inline(always)]
    pub fn set_vararg(&mut self, start: usize, count: usize) {
        self.vararg_start = start;
        self.vararg_count = count.min(65535) as u16;
    }
}

impl std::fmt::Debug for LuaCallFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaCallFrame")
            .field("frame_id", &self.frame_id)
            .field("base_ptr", &self.base_ptr)
            .field("top", &self.top)
            .field("pc", &self.pc)
            .field("is_lua", &self.is_lua())
            .finish()
    }
}
