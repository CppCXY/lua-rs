use crate::LuaFunction;
use crate::LuaValue;
use std::cell::RefCell;

/// 极限优化的 LuaCallFrame: 80 bytes → 64 bytes (20% reduction!)
///
/// 在保持代码兼容性的前提下，通过以下方式压缩：
/// 1. result_reg: usize → u16 (寄存器索引 < 65536)
/// 2. num_results: usize → u16 (返回值数量 < 65535, 0xFFFF = 多值)
/// 3. vararg_count: usize → u16 (可变参数 < 65536)
/// 4. flags: u8 合并布尔标志
///
/// 内存布局（64 bytes，完美对齐）：
/// - 16 bytes: function_value
/// - 40 bytes: base_ptr + top + pc + frame_id + vararg_start (5×usize)
/// - 6 bytes: result_reg + num_results + vararg_count (3×u16)
/// - 1 byte: flags
/// - 1 byte: padding
pub struct LuaCallFrame {
    pub function_value: LuaValue, // 16 bytes - 包含 ID + 指针!
    pub base_ptr: usize,          // 8 bytes - 保持 usize 避免类型转换
    pub top: usize,               // 8 bytes
    pub pc: usize,                // 8 bytes
    pub frame_id: usize,          // 8 bytes
    pub vararg_start: usize,      // 8 bytes
    pub result_reg: u16,          // 2 bytes (was 8) - 寄存器索引
    pub num_results: u16,         // 2 bytes (was 8) - 期望返回数
    pub vararg_count: u16,        // 2 bytes (was 8) - vararg 数量
    flags: u8,                    // 1 byte - 标志位
    _padding: u8,                 // 1 byte - 对齐
                                  // Total: 64 bytes (20% reduction from 80 bytes!)
}

// Flag bits
const FLAG_IS_LUA: u8 = 1 << 0;
const FLAG_IS_PROTECTED: u8 = 1 << 1;

// 特殊值：num_results = 0xFFFF 表示接受多个返回值
const NUM_RESULTS_MULTIPLE: u16 = 0xFFFF;

// 特殊值：result_reg = 0xFFFE 表示不写回register (只用return_values)
const RESULT_REG_NO_WRITEBACK: u16 = 0xFFFE;

impl LuaCallFrame {
    #[inline]
    pub fn new_lua_function(
        frame_id: usize,
        function_value: LuaValue,
        base_ptr: usize,
        max_stack_size: usize,
        result_reg: usize,
        num_results: usize,
    ) -> Self {
        LuaCallFrame {
            function_value,
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

    /// 零开销获取函数指针 - 直接从 LuaValue.secondary 读取!
    #[inline(always)]
    pub fn get_function_ptr(&self) -> Option<*const RefCell<LuaFunction>> {
        if self.is_lua() {
            self.function_value.as_function_ptr()
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
    #[inline(always)]
    pub fn get_function_id(&self) -> Option<crate::object_pool::FunctionId> {
        self.function_value.as_function_id()
    }

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
