# Lua 5.4 完全复刻实施计划

## 阶段 1: 指令集重构 (基于 lopcodes.h)

### 1.1 指令格式
- iABC: C(8) | B(8) | k(1) | A(8) | Op(7)
- iABx: Bx(17) | A(8) | Op(7)
- iAsBx: sBx(signed 17) | A(8) | Op(7)
- iAx: Ax(25) | Op(7)
- isJ: sJ(signed 25) | Op(7)

### 1.2 完整指令集 (83个指令)
1. OP_MOVE - R[A] := R[B]
2. OP_LOADI - R[A] := sBx
3. OP_LOADF - R[A] := (lua_Number)sBx
4. OP_LOADK - R[A] := K[Bx]
5. OP_LOADKX - R[A] := K[extra arg]
6. OP_LOADFALSE - R[A] := false
7. OP_LFALSESKIP - R[A] := false; pc++
8. OP_LOADTRUE - R[A] := true
9. OP_LOADNIL - R[A], R[A+1], ..., R[A+B] := nil
10. OP_GETUPVAL - R[A] := UpValue[B]
11. OP_SETUPVAL - UpValue[B] := R[A]
12. OP_GETTABUP - R[A] := UpValue[B][K[C]:string]
13. OP_GETTABLE - R[A] := R[B][R[C]]
14. OP_GETI - R[A] := R[B][C]
15. OP_GETFIELD - R[A] := R[B][K[C]:string]
16. OP_SETTABUP - UpValue[A][K[B]:string] := RK(C)
17. OP_SETTABLE - R[A][R[B]] := RK(C)
18. OP_SETI - R[A][B] := RK(C)
19. OP_SETFIELD - R[A][K[B]:string] := RK(C)
20. OP_NEWTABLE - R[A] := {}
21. OP_SELF - R[A+1] := R[B]; R[A] := R[B][RK(C)]
22-34. 算术运算 (ADDI, ADDK, SUBK, MULK, MODK, POWK, DIVK, IDIVK, BANDK, BORK, BXORK, SHRI, SHLI)
35-47. 二元运算 (ADD, SUB, MUL, MOD, POW, DIV, IDIV, BAND, BOR, BXOR, SHL, SHR)
48-50. 元方法调用 (MMBIN, MMBINI, MMBINK)
51-54. 一元运算 (UNM, BNOT, NOT, LEN)
55. OP_CONCAT
56-57. OP_CLOSE, OP_TBC
58. OP_JMP
59-61. 比较 (EQ, LT, LE)
62-67. 比较优化 (EQK, EQI, LTI, LEI, GTI, GEI)
68-69. OP_TEST, OP_TESTSET
70-72. 函数调用 (CALL, TAILCALL, RETURN)
73-74. OP_RETURN0, OP_RETURN1
75-76. 数值for循环 (FORLOOP, FORPREP)
77-79. 泛型for循环 (TFORPREP, TFORCALL, TFORLOOP)
80. OP_SETLIST
81. OP_CLOSURE
82. OP_VARARG
83. OP_VARARGPREP
84. OP_EXTRAARG

## 阶段 2: VM核心重构

### 2.1 CallInfo 结构
- 参照 lstate.h 中的 CallInfo
- func: 当前函数
- top: 栈顶
- previous/next: 调用链

### 2.2 指令分发循环
- 参照 lvm.c 中的主循环
- 支持跳转表优化

### 2.3 栈管理
- 动态top管理
- 正确的栈增长/收缩

## 阶段 3: 编译器重构

### 3.1 表达式编译
- discharge: 将表达式结果存入寄存器
- exp2reg, exp2anyreg, exp2nextreg
- 正确的临时寄存器管理

### 3.2 freereg管理
- luaK_reserveregs
- freereg的增长和收缩
- 与nactvar的配合

### 3.3 代码生成
- 使用新的指令格式
- 正确的跳转修正

## 实施步骤

1. ✅ 创建此计划文档
2. ⏳ 重写 opcode/mod.rs - 完整的Lua 5.4指令集
3. ⏳ 重写 dispatcher.rs - VM主循环
4. ⏳ 修改 lua_call_frame.rs - 符合CallInfo结构
5. ⏳ 重构编译器 - freereg/表达式处理
6. ⏳ 测试和验证
