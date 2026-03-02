栈帧大小对比分析
数据对比
指标	我们的 lua_execute	C Lua luaV_execute	倍数
栈帧大小	sub rsp, 1128 (1128B)	sub rsp, 328 (328B)	3.4x
保存的寄存器	8 GPR + 2 XMM	8 GPR + 6 XMM	—
总帧含保存	~1224B	~488B	2.5x
汇编行数	~18000 行	~5460 行	3.3x
CALL 指令数	320	113	2.8x
独立栈槽数	124 个	14 个	8.9x
panic_bounds_check	104 次	0 次	∞
format!	5 次	0 次	∞
dealloc	7 次	0 次	∞
C Lua 寄存器分配（6 个热变量全部在寄存器中）
寄存器	变量	说明
r15	L	lua_State*
rbp	ci	CallInfo*
r12	pc	指令指针（每次 +4）
r13	base	栈基址
rbx	i	当前指令
r14	k	常量数组指针
r9d	trap	hook 标志
只溢出 cl(closure)、k(偶尔重加载) 到栈上，其余全部寄存器访问。

栈帧膨胀根因分析
1. 104 次 panic_bounds_check（最大因素，~400-500B 贡献）
每次安全索引 stack[x]、constants[x] 都产生：

一个条件分支 + panic 路径
LLVM 因此无法确定变量生命周期，无法复用栈槽
在 match 的 80+ 个 arm 中，不同 arm 的边界检查使 LLVM 认为所有路径都可能跳转到 panic，从而保守分配栈空间
优化方案：将热路径中的 stack[x] 改为 unsafe { *stack.get_unchecked(x) }。目前 37% 已用 unchecked，剩余 ~50 处安全索引（主要在算术/比较/SetTable/ForPrep 中）。

2. Fat pointer 变量过多（~100B 贡献）
code: &Vec<Instruction> — 8B 指针 (但 Vec 本身是 ptr+len+cap = 24B，访问时需解引用两层)
constants: &Vec<LuaValue> — 同上
upvalue_ptrs: &[UpvaluePtr] — fat pointer 16B (ptr + len)
C Lua 只用一个 cl 指针，通过 cl->p->code、cl->p->k 间接访问
优化方案：将 code、constants 从 &Vec<T> 改为 raw pointer + len，或直接存 *const Instruction、*const LuaValue 裸指针（配合 get_unchecked），省去 Vec 的 capacity 字段和二次解引用。

3. format! 在错误路径内联（~150B 贡献）
format!() 在栈上创建 fmt::Arguments 结构（~72B），即使在 never-taken 的错误路径中，LLVM 通常仍为其预留帧空间。5 次 format! 调用如果不能共享栈空间，可能占用 150-360B。

优化方案：将含 format! 的错误路径提取为 #[cold] #[inline(never)] 函数。

4. match arm 过大导致 LLVM 栈布局保守（~200B 贡献）
80+ 个 opcode arm 的局部变量理论上不需同时存在（union 语义），LLVM 通常能优化这一点。但当 arm 中有 ? 错误传播、bounds check panic、格式化等复杂控制流时，LLVM 对栈空间的重叠判断变得保守。

最大的三个 arm：

OpCode::Call (~195 行) — 内联了 Lua 直调 + __call 路径
OpCode::ForPrep (~106 行) — 含 forlimit 复杂分支
OpCode::Return1 内联路径 — 含帧弹出逻辑
优化方案：对最大的几个 arm，将冷路径（__call、forlimit 错误、ForPrep 字符串转换）提取为 #[cold] #[inline(never)] 函数。

5. SEH 异常处理开销（Windows 特有）
__CxxFrameHandler3 处理器 + 9 个 dtor cleanup 块。每个需要保存/恢复状态的帧入口会增加帧大小。

优化方案：使用 panic=abort 编译可消除 SEH 开销（可能节省 ~50-100B），但会改变语义。

优化优先级排序
优先级	优化方案	预期收益	难度
P0	将热路径 stack[x]/constants[x] 改为 get_unchecked	300-500B 帧缩减，消除 ~60 次 bounds check	中（需 audit 每处安全性）
P1	将 code/constants 改为 raw pointer *const T	50-100B + 减少间接寻址	低
P2	将含 format! 的错误路径提取为 #[cold] fn	100-200B	低
P3	将 __call 处理提取为 #[inline(never)] 函数	50-100B + 改善热路径代码布局	中
P4	使用 panic=abort 消除 SEH	50-100B	低（但有语义影响）
P0 + P1 + P2 合计可能将栈帧从 1128B 降低到 ~500-600B，接近 C Lua 的 328B。