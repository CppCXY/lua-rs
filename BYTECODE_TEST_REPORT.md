# 字节码对比测试报告

## 测试结果概览

- **总文件数**: 55
- **通过**: 2 (db.lua, run_test.lua)
- **失败**: 53
- **跳过**: 0

## 主要问题类别

### 1. 指令数量差异
大多数文件的指令数量都比官方多 5-15%,说明我们生成了额外的指令。

常见原因:
- 寄存器分配不够优化
- 缺少某些常量优化(特别是nil)
- 表达式编译产生临时值

###  2. 已识别的具体问题

#### 问题 A: nil 常量未优化
**现象**: `package.loaded["test"] = nil` 生成额外的 LOADNIL 指令

**官方字节码**:
```
GETTABUP  0 0 0   ; _ENV "package"
GETFIELD  0 0 1   ; "loaded"
SETFIELD  0 2 3k  ; "test" nil (k=1表示nil是常量)
```

**我们的字节码**:
```
LOADNIL 0 0        ; 多余!
GETTABUP 1 0 0
GETFIELD 1 1 1
SETFIELD 1 2 0     ; k=0表示值来自寄存器
```

**已尝试的修复**:
- ✅ 在 `try_expr_as_constant` 中添加了 nil 识别
- ❌ 在 `compile_assign_stat` 中添加了 `table[string] = constant` 优化,但未生效

**问题分析**:
- Parser 可能将 `table["string"]` 解析为不同的 IndexKey 类型
- 需要检查 AST 结构并相应调整优化逻辑

#### 问题 B: 指令名称格式问题(已部分修复)
- ✅ SETLIST, SETI, GETI, EQK 已修复为大写
- ❌ 位运算指令仍然使用 Debug 格式输出 (BAnd -> BAND等)
- ❌ LOADF 指令参数显示格式不正确

### 3. 通过的测试文件
- `db.lua` - Debug 库测试
- `run_test.lua` - 测试运行器

这两个文件可能结构简单或使用了我们已优化的特性。

### 4. 失败模式分析

#### 轻微失败 (指令数差1-3条)
- test_bw.lua: 65 vs 66
- test_capture.lua: 32 vs 33
- test_env2.lua: 35 vs 37
- tracegc.lua: 51 vs 53

这些可能只需要修复几个特定的优化问题。

#### 中度失败 (指令数差10-100条)
- big.lua: 290 vs 318
- bwcoercion.lua: 189 vs 218  
- cstack.lua: 457 vs 477
- errors.lua: 2094 vs 2136

需要系统性地修复常见模式。

#### 重度失败 (指令数差100+条)
- api.lua: 6325 vs 6710 (+385)
- attrib.lua: 2233 vs 2723 (+490)
- calls.lua: 2021 vs 2407 (+386)
- coroutine.lua: 4818 vs 5577 (+759)
- vararg.lua: 699 vs 875 (+176)

这些大文件可能暴露了多个系统性问题的累积效应。

### 5. 编译错误
- **math.lua**: 编译失败 (需要调查原因)
- **strings.lua**: UTF-8编码问题

## 下一步建议

### 短期(修复常见模式)
1. **修复 nil 常量优化** - 这影响很多测试
   - 正确识别 `table[string_literal] = nil` 模式
   - 确保 Parser 返回的 IndexKey 类型被正确处理
   
2. **修复指令格式输出** - 便于调试
   - 添加所有位运算指令的大写格式
   - 修复 LOADF 参数显示

3. **分析简单失败案例**
   - 从 test_bw.lua (只差1条指令) 开始
   - 找出差异的根本原因
   - 应用修复到相似模式

### 中期(优化寄存器分配)
1. 减少临时寄存器使用
2. 改进表达式编译的目标寄存器指定
3. 更激进的常量折叠

### 长期(系统性改进)
1. 实现与 Lua 5.4 完全一致的寄存器分配算法
2. 添加更多的 RK (Register/Konstant) 优化
3. 实现窥孔优化(peephole optimization)

## 工具和资源

- **对比脚本**: `compare_all_testes.ps1`
- **输出目录**: `bytecode_comparison_output/`
- **测试目录**: `lua_tests/testes/`

使用以下命令查看特定差异:
```powershell
code --diff "bytecode_comparison_output\<file>_official.txt" "bytecode_comparison_output\<file>_ours.txt"
```
